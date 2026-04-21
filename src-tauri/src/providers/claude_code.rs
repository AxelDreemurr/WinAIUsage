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
    pub is_peak_hours: bool,
    pub peak_status: String,
}

impl ClaudeCodeData {
    pub fn loading() -> Self {
        let (is_peak, peak_status) = compute_peak_status();
        Self {
            is_available: false,
            status_line: crate::t("Cargando...", "Loading...").to_string(),
            five_hour: None,
            seven_day: None,
            daily_tokens: 0,
            daily_cost: 0.0,
            error: None,
            is_peak_hours: is_peak,
            peak_status,
        }
    }

    fn unavailable(error: impl Into<String>) -> Self {
        let (is_peak, peak_status) = compute_peak_status();
        Self {
            is_available: false,
            status_line: String::new(),
            five_hour: None,
            seven_day: None,
            daily_tokens: 0,
            daily_cost: 0.0,
            error: Some(error.into()),
            is_peak_hours: is_peak,
            peak_status,
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
        Err(_) => return ClaudeCodeData::unavailable(crate::t("USERPROFILE no encontrado", "USERPROFILE not found")),
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

    let status_line = if crate::is_es() {
        format!("{} tokens · ${:.2} hoy", fmt_number(daily_tokens), daily_cost)
    } else {
        format!("{} tokens · ${:.2} today", fmt_number(daily_tokens), daily_cost)
    };

    let (is_peak_hours, peak_status) = compute_peak_status();

    ClaudeCodeData {
        is_available: true,
        status_line,
        five_hour,
        seven_day,
        daily_tokens,
        daily_cost,
        error: None,
        is_peak_hours,
        peak_status,
    }
}

// ── Step 1: read token ───────────────────────────────────────────────────────

fn read_token(path: &PathBuf) -> Result<String, String> {
    let content =
        std::fs::read_to_string(path).map_err(|_| crate::t("Claude Code no encontrado", "Claude Code not found").to_string())?;
    let creds: Credentials =
        serde_json::from_str(&content).map_err(|_| crate::t("Error leyendo credenciales", "Error reading credentials").to_string())?;
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

/// Returns the nth weekday (0=Sun..6=Sat) of a given month/year (1-indexed ordinal).
/// e.g. nth_weekday(2025, 3, 0, 2) → second Sunday of March 2025 (day-of-month).
fn nth_weekday(year: i32, month: u32, weekday: u32, nth: u32) -> u32 {
    // Find first occurrence of `weekday` in month
    // chrono weekday: Mon=1..Sun=7, but we work with 0=Sun..6=Sat mapping below
    use chrono::Datelike;
    let first = chrono::NaiveDate::from_ymd_opt(year, month, 1).unwrap();
    // chrono: Monday=0..Sunday=6 for num_days_from_monday()
    // We need: Sunday=0, Monday=1 … Saturday=6
    let first_dow = (first.weekday().num_days_from_sunday()) as u32; // Sun=0
    let days_until = (7 + weekday - first_dow) % 7;
    1 + days_until + (nth - 1) * 7
}

/// Returns true if the UTC timestamp is during PDT (Pacific Daylight Time, UTC-7).
/// PDT runs from the second Sunday of March at 2:00 AM PST
/// to the first Sunday of November at 2:00 AM PDT.
fn is_pdt(utc_now: chrono::DateTime<chrono::Utc>) -> bool {
    use chrono::Datelike;
    let year = utc_now.year();
    let month = utc_now.month();

    if month < 3 || month > 11 {
        return false; // Jan, Feb, Dec → always PST
    }
    if month > 3 && month < 11 {
        return true; // Apr–Oct → always PDT
    }

    if month == 3 {
        // PDT starts: second Sunday of March at 2:00 AM PST (= 10:00 UTC)
        let start_day = nth_weekday(year, 3, 0, 2);
        let start_utc_hour = 10u32; // 2 AM PST = UTC-8 → 10:00 UTC
        let transition = chrono::NaiveDate::from_ymd_opt(year, 3, start_day)
            .unwrap()
            .and_hms_opt(start_utc_hour, 0, 0)
            .unwrap()
            .and_utc();
        return utc_now >= transition;
    }

    // month == 11
    // PDT ends: first Sunday of November at 2:00 AM PDT (= 09:00 UTC)
    let end_day = nth_weekday(year, 11, 0, 1);
    let end_utc_hour = 9u32; // 2 AM PDT = UTC-7 → 09:00 UTC
    let transition = chrono::NaiveDate::from_ymd_opt(year, 11, end_day)
        .unwrap()
        .and_hms_opt(end_utc_hour, 0, 0)
        .unwrap()
        .and_utc();
    utc_now < transition
}

/// Computes whether we're in Claude Code peak hours (Mon–Fri, 5 AM–11 AM PT)
/// and returns (is_peak, peak_status_string).
fn compute_peak_status() -> (bool, String) {
    use chrono::{Datelike, Timelike};
    let now_utc = chrono::Utc::now();
    let offset_hours: i64 = if is_pdt(now_utc) { -7 } else { -8 };
    let now_pt = now_utc + chrono::Duration::hours(offset_hours);

    let weekday = now_pt.weekday(); // Mon=Monday … Sun=Sunday
    let is_weekend = matches!(weekday, chrono::Weekday::Sat | chrono::Weekday::Sun);
    let hour = now_pt.hour(); // 0..23

    if is_weekend {
        return (false, crate::t("Off-Peak (fin de semana)", "Off-Peak (weekend)").to_string());
    }

    // Peak = 5:00 AM (inclusive) to 11:00 AM (exclusive)
    let is_peak = hour >= 5 && hour < 11;
    if is_peak {
        (true, "Peak".to_string())
    } else {
        (false, "Off-Peak".to_string())
    }
}

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
