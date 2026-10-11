use super::claude_desktop_auth::{self, DesktopAuthError};
use crate::pricing::{self, DailyUsage, TokenUsage};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

type Quota = (Option<UsagePeriod>, Option<UsagePeriod>);

/// Last successful quota response plus the API's back-off deadline. Kept in memory and
/// mirrored to disk so transient failures (e.g. 429) — even right after a relaunch —
/// don't wipe the bars from the UI.
#[derive(Serialize, Deserialize, Clone, Default)]
struct QuotaCache {
    five_hour: Option<UsagePeriod>,
    seven_day: Option<UsagePeriod>,
    /// Unix seconds until which the usage API must not be called.
    backoff_until: Option<u64>,
}

static CACHE: Mutex<Option<QuotaCache>> = Mutex::new(None);

fn cache_path() -> PathBuf {
    crate::settings::get_settings_path().with_file_name("claude_quota_cache.json")
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn with_cache<R>(f: impl FnOnce(&mut QuotaCache) -> R) -> R {
    let mut g = CACHE.lock().unwrap();
    let cache = g.get_or_insert_with(|| {
        std::fs::read_to_string(cache_path())
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_default()
    });
    f(cache)
}

fn save_cache(cache: &QuotaCache) {
    if let Ok(json) = serde_json::to_string(cache) {
        let _ = std::fs::write(cache_path(), json);
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct UsagePeriod {
    pub utilization: f64,
    pub resets_at: String,
}

#[derive(Serialize, Clone)]
pub struct ClaudeCodeData {
    pub is_available: bool,
    pub status_line: String,
    pub five_hour: Option<UsagePeriod>,
    pub seven_day: Option<UsagePeriod>,
    /// Today's tokens and API-equivalent cost, from the local JSONL logs.
    pub today: Option<DailyUsage>,
    pub error: Option<String>,
}

impl ClaudeCodeData {
    pub fn loading() -> Self {
        Self {
            is_available: false,
            status_line: crate::t("Cargando...", "Loading...").to_string(),
            five_hour: None,
            seven_day: None,
            today: None,
            error: None,
        }
    }

    fn unavailable(error: impl Into<String>) -> Self {
        Self {
            is_available: false,
            status_line: String::new(),
            five_hour: None,
            seven_day: None,
            today: None,
            error: Some(error.into()),
        }
    }
}

// ── Deserialize helpers ──────────────────────────────────────────────────────

#[derive(Deserialize)]
struct Credentials {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: OAuthData,
}

#[derive(Deserialize)]
struct OAuthData {
    #[serde(rename = "accessToken")]
    access_token: String,
    /// Unix ms; 0 when unknown.
    #[serde(rename = "expiresAt", default)]
    expires_at: f64,
    /// Plan stamped by Claude Code at sign-in ("pro", "max", ...).
    #[serde(rename = "subscriptionType")]
    subscription_type: Option<String>,
    /// e.g. "default_claude_max_20x"; distinguishes the Max tiers.
    #[serde(rename = "rateLimitTier")]
    rate_limit_tier: Option<String>,
}

#[derive(Deserialize)]
struct ApiResponse {
    five_hour: Option<ApiPeriod>,
    seven_day: Option<ApiPeriod>,
}

#[derive(Deserialize)]
struct ApiPeriod {
    utilization: f64,
    resets_at: Option<String>,
}

// ── Entry point ──────────────────────────────────────────────────────────────

pub async fn get_data() -> ClaudeCodeData {
    let userprofile = match std::env::var("USERPROFILE") {
        Ok(v) => v,
        Err(_) => {
            return ClaudeCodeData::unavailable(crate::t(
                "USERPROFILE no encontrado",
                "USERPROFILE not found",
            ))
        }
    };

    // Local logs don't depend on the quota API, so today's usage shows even when it fails.
    let projects_dir = PathBuf::from(&userprofile).join(".claude").join("projects");
    let today = tokio::task::spawn_blocking(move || read_usage(&projects_dir, pricing::today()))
        .await
        .ok();

    let mut data = fetch_status(&userprofile).await;
    data.today = today;
    data
}

async fn fetch_status(userprofile: &str) -> ClaudeCodeData {
    let account = read_oauth_account(userprofile);

    let session = match resolve_session(userprofile, account.as_ref()) {
        Ok(s) => s,
        Err(e) => return ClaudeCodeData::unavailable(e),
    };

    let (five_hour, seven_day) = match fetch_quota(&session.access_token, session.source).await {
        Ok(res) => res,
        Err(e) => return ClaudeCodeData::unavailable(e),
    };

    // Claude Code refreshes its cached profile in ~/.claude.json, so it reflects plan
    // changes; the session only holds the plan stamped at sign-in.
    let (subscription, tier) = account
        .as_ref()
        .and_then(|a| {
            Some((
                a["organizationType"].as_str()?,
                a["organizationRateLimitTier"].as_str().unwrap_or_default(),
            ))
        })
        .map(|(s, t)| (s.to_string(), t.to_string()))
        .unwrap_or((
            session.subscription_type.unwrap_or_default(),
            session.rate_limit_tier.unwrap_or_default(),
        ));
    let plan = format_plan(&subscription, &tier);
    let status_line = format!("{} · {}", plan, crate::t("Sesión activa", "Active session"));

    ClaudeCodeData {
        is_available: true,
        status_line,
        five_hour,
        seven_day,
        today: None,
        error: None,
    }
}

// ── Step 1: find a session ────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum SessionSource {
    Cli,
    Desktop,
}

struct Session {
    access_token: String,
    source: SessionSource,
    subscription_type: Option<String>,
    rate_limit_tier: Option<String>,
}

/// Claude Code's CLI keeps its login in `~/.claude/.credentials.json`; when Claude Code
/// runs from Claude Desktop that file stays empty and the login lives in Desktop.
fn resolve_session(
    userprofile: &str,
    account: Option<&serde_json::Value>,
) -> Result<Session, String> {
    let creds_path = PathBuf::from(userprofile)
        .join(".claude")
        .join(".credentials.json");
    if let Some(oauth) = read_cli_credentials(&creds_path) {
        return Ok(Session {
            access_token: oauth.access_token,
            source: SessionSource::Cli,
            subscription_type: oauth.subscription_type,
            rate_limit_tier: oauth.rate_limit_tier,
        });
    }

    let field = |name: &str| account.and_then(|a| a[name].as_str());
    match claude_desktop_auth::read_token(field("organizationUuid"), field("accountUuid")) {
        Ok(t) => Ok(Session {
            access_token: t.access_token,
            source: SessionSource::Desktop,
            subscription_type: t.subscription_type,
            rate_limit_tier: t.rate_limit_tier,
        }),
        Err(DesktopAuthError::Stale) => Err(crate::t(
            "Sesión de Claude Desktop expirada. Ábrelo para renovarla",
            "Claude Desktop session expired. Open it to renew",
        )
        .to_string()),
        Err(DesktopAuthError::Invalid) => Err(crate::t(
            "No se pudo leer la sesión de Claude Desktop",
            "Couldn't read the Claude Desktop session",
        )
        .to_string()),
        // Without a token the usage API answers 429, which would read as "no data".
        Err(DesktopAuthError::NotFound) => Err(crate::t(
            "Inicia sesión en Claude Desktop o ejecuta 'claude login'",
            "Sign in to Claude Desktop or run 'claude login'",
        )
        .to_string()),
    }
}

/// The CLI login, if it holds a token that hasn't expired.
fn read_cli_credentials(path: &PathBuf) -> Option<OAuthData> {
    let creds: Credentials = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    let oauth = creds.claude_ai_oauth;
    let now_ms = chrono::Utc::now().timestamp_millis() as f64;
    let valid = !oauth.access_token.trim().is_empty()
        && (oauth.expires_at == 0.0 || oauth.expires_at > now_ms);
    valid.then_some(oauth)
}

/// Account profile Claude Code caches (and refreshes) in `~/.claude.json`.
fn read_oauth_account(userprofile: &str) -> Option<serde_json::Value> {
    let content = std::fs::read_to_string(PathBuf::from(userprofile).join(".claude.json")).ok()?;
    let mut v: serde_json::Value = serde_json::from_str(&content).ok()?;
    Some(v["oauthAccount"].take()).filter(|a| a.is_object())
}

/// "pro" / "claude_pro" -> "Pro"; "claude_max" + "default_claude_max_20x" -> "Max 20x".
fn format_plan(subscription: &str, tier: &str) -> String {
    let subscription = subscription.trim();
    let subscription = subscription.strip_prefix("claude_").unwrap_or(subscription);
    let mut chars = subscription.chars();
    let Some(first) = chars.next() else {
        return "Claude".to_string();
    };
    let mut plan = first.to_uppercase().chain(chars).collect::<String>();
    if let Some(multiplier) = tier
        .rsplit('_')
        .next()
        .filter(|m| m.ends_with('x') && m.len() > 1)
    {
        if multiplier[..multiplier.len() - 1]
            .chars()
            .all(|c| c.is_ascii_digit())
        {
            plan = format!("{plan} {multiplier}");
        }
    }
    plan
}

#[cfg(test)]
mod tests {
    use super::format_plan;

    #[test]
    fn formats_claude_plans() {
        assert_eq!(format_plan("pro", "default_claude_ai"), "Pro");
        assert_eq!(format_plan("claude_pro", "default_claude_ai"), "Pro");
        assert_eq!(
            format_plan("claude_max", "default_claude_max_20x"),
            "Max 20x"
        );
        assert_eq!(format_plan("claude_team", ""), "Team");
        assert_eq!(format_plan("max", "default_claude_max_20x"), "Max 20x");
        assert_eq!(format_plan("max", "default_claude_max_5x"), "Max 5x");
        assert_eq!(format_plan("", ""), "Claude");
    }
}

// ── Step 2: quota API ────────────────────────────────────────────────────────

async fn fetch_quota(token: &str, source: SessionSource) -> Result<Quota, String> {
    let cached = || with_cache(|c| (c.five_hour.clone(), c.seven_day.clone()));

    if with_cache(|c| c.backoff_until.is_some_and(|until| unix_now() < until)) {
        return Ok(cached());
    }

    let client = match reqwest::Client::builder().build() {
        Ok(c) => c,
        Err(_) => return Ok(cached()),
    };

    let result = client
        .get("https://api.anthropic.com/api/oauth/usage")
        .header("Authorization", format!("Bearer {}", token))
        .header("anthropic-beta", "oauth-2025-04-20")
        .timeout(std::time::Duration::from_secs(8))
        .send()
        .await;

    let resp = match result {
        Ok(r) if r.status().is_success() => r,
        Ok(r) if r.status().as_u16() == 401 || r.status().as_u16() == 403 => {
            return Err(match source {
                SessionSource::Cli => crate::t(
                    "Sesión expirada. Ejecuta 'claude login'",
                    "Session expired. Run 'claude login'",
                ),
                SessionSource::Desktop => crate::t(
                    "Sesión de Claude Desktop expirada. Ábrelo para renovarla",
                    "Claude Desktop session expired. Open it to renew",
                ),
            }
            .to_string());
        }
        Ok(r) if r.status().as_u16() == 429 => {
            let secs = r
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok())
                .unwrap_or(600);
            with_cache(|c| {
                c.backoff_until = Some(unix_now() + secs);
                save_cache(c);
            });
            return Ok(cached());
        }
        _ => return Ok(cached()),
    };

    let data: ApiResponse = match resp.json().await {
        Ok(d) => d,
        Err(_) => return Ok(cached()),
    };

    let five = data.five_hour.map(|p| UsagePeriod {
        utilization: p.utilization,
        resets_at: p.resets_at.unwrap_or_default(),
    });
    let seven = data.seven_day.map(|p| UsagePeriod {
        utilization: p.utilization,
        resets_at: p.resets_at.unwrap_or_default(),
    });

    with_cache(|c| {
        *c = QuotaCache {
            five_hour: five.clone(),
            seven_day: seven.clone(),
            backoff_until: None,
        };
        save_cache(c);
    });
    Ok((five, seven))
}

// ── Step 3: JSONL local ──────────────────────────────────────────────────────

/// Sums one local day's assistant messages across all Claude Code transcripts.
///
/// Claude Code writes one JSONL entry per content block, so a single API response
/// appears several times with identical usage; entries are deduplicated by
/// message id + request id, as Claude Code's own `/cost` does.
fn read_usage(projects_dir: &std::path::Path, day: chrono::NaiveDate) -> DailyUsage {
    let mut seen = std::collections::HashSet::new();
    let mut usage = DailyUsage::default();

    for entry in walkdir::WalkDir::new(projects_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "jsonl"))
        .filter(|e| pricing::modified_since(e.path(), day))
    {
        let content = match std::fs::read_to_string(entry.path()) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for line in content.lines() {
            // Cheap pre-filter: most lines are user messages and tool results.
            if !line.contains("\"usage\"") {
                continue;
            }
            let v: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if v["type"].as_str() != Some("assistant") {
                continue;
            }
            let date = v["timestamp"].as_str().and_then(pricing::local_date_of);
            if date != Some(day) {
                continue;
            }

            let message = &v["message"];
            let key = format!(
                "{}:{}",
                message["id"].as_str().unwrap_or(""),
                v["requestId"].as_str().unwrap_or("")
            );
            if key != ":" && !seen.insert(key) {
                continue;
            }

            let u = &message["usage"];
            let n = |field: &str| u[field].as_u64().unwrap_or(0);
            let cache_write = n("cache_creation_input_tokens");
            // Older logs lack the TTL breakdown; those writes were all 5-minute.
            let cache_write_1h = u["cache_creation"]["ephemeral_1h_input_tokens"]
                .as_u64()
                .unwrap_or(0)
                .min(cache_write);
            let tokens = TokenUsage {
                input: n("input_tokens"),
                output: n("output_tokens"),
                cache_write_5m: cache_write - cache_write_1h,
                cache_write_1h,
                cache_read: n("cache_read_input_tokens"),
            };

            let model = message["model"].as_str().unwrap_or("unknown");
            let price = pricing::claude(
                model,
                tokens.prompt(),
                u["speed"].as_str() == Some("fast"),
                u["inference_geo"].as_str() == Some("us"),
            );
            usage.add(model, &tokens, price);
        }
    }

    usage
}
