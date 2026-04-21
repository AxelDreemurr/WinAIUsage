use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Clone)]
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
    pub daily_tokens: u64,
    pub daily_cost: f64,
    pub error: Option<String>,
}

impl ClaudeCodeData {
    pub fn loading() -> Self {
        Self {
            is_available: false,
            status_line: "Cargando...".to_string(),
            five_hour: None,
            seven_day: None,
            daily_tokens: 0,
            daily_cost: 0.0,
            error: None,
        }
    }

    fn unavailable(error: impl Into<String>) -> Self {
        Self {
            is_available: false,
            status_line: String::new(),
            five_hour: None,
            seven_day: None,
            daily_tokens: 0,
            daily_cost: 0.0,
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
        Err(_) => return ClaudeCodeData::unavailable("USERPROFILE no encontrado"),
    };

    let creds_path = PathBuf::from(&userprofile)
        .join(".claude")
        .join(".credentials.json");

    let token = match read_token(&creds_path) {
        Ok(t) => t,
        Err(e) => return ClaudeCodeData::unavailable(e),
    };

    let (five_hour, seven_day) = fetch_quota(&token).await;
    let (daily_tokens, daily_cost) = read_daily_jsonl(&userprofile);

    let status_line = format!(
        "{} tokens · ${:.2} hoy",
        fmt_number(daily_tokens),
        daily_cost
    );

    ClaudeCodeData {
        is_available: true,
        status_line,
        five_hour,
        seven_day,
        daily_tokens,
        daily_cost,
        error: None,
    }
}

// ── Step 1: read token ───────────────────────────────────────────────────────

fn read_token(path: &PathBuf) -> Result<String, String> {
    let content = std::fs::read_to_string(path)
        .map_err(|_| "Claude Code no encontrado".to_string())?;
    let creds: Credentials = serde_json::from_str(&content)
        .map_err(|_| "Error leyendo credenciales".to_string())?;
    Ok(creds.claude_ai_oauth.access_token)
}

// ── Step 2: quota API ────────────────────────────────────────────────────────

async fn fetch_quota(token: &str) -> (Option<UsagePeriod>, Option<UsagePeriod>) {
    let client = match reqwest::Client::builder().build() {
        Ok(c) => c,
        Err(_) => return (None, None),
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
        _ => return (None, None),
    };

    let data: ApiResponse = match resp.json().await {
        Ok(d) => d,
        Err(_) => return (None, None),
    };

    let five = data.five_hour.map(|p| UsagePeriod {
        utilization: p.utilization,
        resets_at: p.resets_at.unwrap_or_default(),
    });
    let seven = data.seven_day.map(|p| UsagePeriod {
        utilization: p.utilization,
        resets_at: p.resets_at.unwrap_or_default(),
    });

    (five, seven)
}

// ── Step 3: JSONL local ──────────────────────────────────────────────────────

fn read_daily_jsonl(userprofile: &str) -> (u64, f64) {
    let projects_dir = PathBuf::from(userprofile).join(".claude").join("projects");
    let today = Utc::now().format("%Y-%m-%d").to_string();

    let mut total_tokens: u64 = 0;
    let mut total_cost: f64 = 0.0;

    for entry in walkdir::WalkDir::new(&projects_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.path()
                .extension()
                .map(|ext| ext == "jsonl")
                .unwrap_or(false)
        })
    {
        let content = match std::fs::read_to_string(entry.path()) {
            Ok(c) => c,
            Err(_) => continue,
        };

        for line in content.lines() {
            let v: serde_json::Value = match serde_json::from_str(line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            if v["type"].as_str() != Some("assistant") {
                continue;
            }

            let timestamp = v["timestamp"].as_str().unwrap_or("");
            if !timestamp.starts_with(&today) {
                continue;
            }

            let usage = &v["message"]["usage"];
            let input = usage["input_tokens"].as_u64().unwrap_or(0);
            let output = usage["output_tokens"].as_u64().unwrap_or(0);
            let cache_read = usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
            let cache_creation = usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);

            total_tokens += input + output + cache_read + cache_creation;

            let model = v["message"]["model"].as_str().unwrap_or("claude-sonnet");
            let (price_in, price_out) = model_pricing(model);
            let price_cache_read = price_in * 0.1;
            let price_cache_create = price_in * 0.25;

            total_cost += (input as f64 * price_in
                + output as f64 * price_out
                + cache_read as f64 * price_cache_read
                + cache_creation as f64 * price_cache_create)
                / 1_000_000.0;
        }
    }

    (total_tokens, total_cost)
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn model_pricing(model: &str) -> (f64, f64) {
    if model.contains("opus") {
        (15.0, 75.0)
    } else if model.contains("haiku") {
        (0.80, 4.0)
    } else {
        (3.0, 15.0)
    }
}

fn fmt_number(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut result = String::with_capacity(s.len() + s.len() / 3);
    for (i, &b) in bytes.iter().enumerate() {
        let pos_from_right = s.len() - 1 - i;
        if i > 0 && pos_from_right % 3 == 2 {
            result.push(',');
        }
        result.push(b as char);
    }
    result
}
