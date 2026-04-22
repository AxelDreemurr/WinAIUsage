use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Serialize, Clone)]
pub struct UsagePeriod {
    pub label_key: String,
    pub utilization: f64,
    pub resets_at: String, // ISO timestamp or empty
}

#[derive(Serialize, Clone)]
pub struct CodexData {
    pub is_available: bool,
    pub status_line: String,
    pub plan_type: String,
    pub periods: Vec<UsagePeriod>,
    pub error: Option<String>,
}

impl CodexData {
    pub fn loading() -> Self {
        Self {
            is_available: false,
            status_line: crate::t("Cargando...", "Loading...").to_string(),
            plan_type: String::new(),
            periods: vec![],
            error: None,
        }
    }

    pub fn unavailable(error: impl Into<String>) -> Self {
        Self {
            is_available: false,
            status_line: String::new(),
            plan_type: String::new(),
            periods: vec![],
            error: Some(error.into()),
        }
    }
}

// JSON parsing structures
#[derive(Deserialize)]
struct AuthFile {
    account_id: Option<String>,
    tokens: AuthTokens,
}

#[derive(Deserialize)]
struct AuthTokens {
    access_token: String,
    account_id: Option<String>,
}

#[derive(Deserialize)]
struct UsagePayload {
    plan_type: Option<String>,
    rate_limit: Option<UsageLimitDetails>,
    code_review_rate_limit: Option<UsageLimitDetails>,
    rate_limit_status: Option<UsageRateLimitStatus>,
}

#[derive(Deserialize)]
struct UsageRateLimitStatus {
    plan_type: Option<String>,
    rate_limit: Option<UsageLimitDetails>,
    code_review_rate_limit: Option<UsageLimitDetails>,
}

#[derive(Deserialize)]
struct UsageLimitDetails {
    primary_window: Option<UsageWindowInfo>,
    secondary_window: Option<UsageWindowInfo>,
    primary: Option<UsageWindowInfo>,
    secondary: Option<UsageWindowInfo>,
}

#[derive(Deserialize)]
struct UsageWindowInfo {
    used_percent: Option<f64>,
    remaining_percent: Option<f64>,
    reset_at: Option<i64>,
    resets_at: Option<i64>,
    reset_after_seconds: Option<i64>,
    window_minutes: Option<i64>,
    limit_window_seconds: Option<i64>,
}

fn resolve_percent(info: &UsageWindowInfo) -> Option<f64> {
    if let Some(used) = info.used_percent {
        Some(used.clamp(0.0, 100.0))
    } else if let Some(remaining) = info.remaining_percent {
        Some((100.0 - remaining).clamp(0.0, 100.0))
    } else {
        None
    }
}

fn resolve_resets_at(info: &UsageWindowInfo) -> String {
    let ts = if let Some(t) = info.reset_at {
        t
    } else if let Some(t) = info.resets_at {
        t
    } else if let Some(secs) = info.reset_after_seconds {
        chrono::Utc::now().timestamp() + secs
    } else {
        0
    };

    if ts > 0 {
        if let Some(dt) = chrono::DateTime::from_timestamp(ts, 0) {
            return dt.to_rfc3339();
        }
    }
    String::new()
}

fn window_to_period(window: Option<&UsageWindowInfo>) -> Option<UsagePeriod> {
    let w = window?;
    let utilization = resolve_percent(w)?;
    
    let mut minutes = w.window_minutes.unwrap_or(0);
    if minutes == 0 {
        if let Some(secs) = w.limit_window_seconds {
            minutes = secs / 60;
        }
    }
    
    let label_key = if minutes >= 10000 {
        "codexWeek".to_string()
    } else {
        "codexSession".to_string()
    };

    Some(UsagePeriod {
        label_key,
        utilization,
        resets_at: resolve_resets_at(w),
    })
}

pub async fn get_data() -> CodexData {
    let userprofile = match std::env::var("USERPROFILE") {
        Ok(v) => v,
        Err(_) => return CodexData::unavailable(crate::t("USERPROFILE no encontrado", "USERPROFILE not found")),
    };

    let auth_path = PathBuf::from(&userprofile)
        .join(".codex")
        .join("auth.json");

    let content = match std::fs::read_to_string(&auth_path) {
        Ok(c) => c,
        Err(_) => return CodexData::unavailable(crate::t("Codex CLI no configurado", "Codex CLI not configured")),
    };

    let auth: AuthFile = match serde_json::from_str(&content) {
        Ok(a) => a,
        Err(_) => return CodexData::unavailable(crate::t("Error leyendo credenciales", "Error reading credentials")),
    };

    if auth.tokens.access_token.trim().is_empty() {
        return CodexData::unavailable(crate::t("Sesión de Codex no iniciada", "Codex session not started"));
    }

    let client = match reqwest::Client::builder().build() {
        Ok(c) => c,
        Err(_) => return CodexData::unavailable("HTTP client error"),
    };

    let mut req = client.get("https://chatgpt.com/backend-api/wham/usage")
        .header("Authorization", format!("Bearer {}", auth.tokens.access_token))
        .header("Accept", "application/json")
        .header("User-Agent", "codex-cli")
        .timeout(std::time::Duration::from_secs(8));

    let account_id = auth.tokens.account_id.or(auth.account_id).unwrap_or_default();
    if !account_id.is_empty() {
        req = req.header("ChatGPT-Account-Id", account_id);
    }

    let resp = match req.send().await {
        Ok(r) => r,
        Err(_) => return CodexData::unavailable(crate::t("Error de red", "Network error")),
    };

    if resp.status().as_u16() == 401 || resp.status().as_u16() == 403 {
        return CodexData::unavailable(crate::t("Sesión expirada. Ejecuta 'codex login'", "Session expired. Run 'codex login'"));
    }

    if !resp.status().is_success() {
        return CodexData::unavailable(crate::t("Error en API de Codex", "Codex API error"));
    }

    let payload: UsagePayload = match resp.json().await {
        Ok(p) => p,
        Err(_) => return CodexData::unavailable(crate::t("Error parseando respuesta de la API", "Error parsing API response")),
    };

    let mut plan_type = payload.plan_type.unwrap_or_else(|| "Codex".to_string());
    let mut periods = vec![];

    let mut add_window = |window: Option<&UsageWindowInfo>| {
        if let Some(p) = window_to_period(window) {
            periods.push(p);
        }
    };

    if let Some(status) = payload.rate_limit_status {
        if let Some(p) = status.plan_type {
            if !p.is_empty() {
                plan_type = p;
            }
        }
        if let Some(rl) = status.rate_limit {
            add_window(rl.primary_window.as_ref().or(rl.primary.as_ref()));
            add_window(rl.secondary_window.as_ref().or(rl.secondary.as_ref()));
        }
        if let Some(crl) = status.code_review_rate_limit {
            add_window(crl.primary_window.as_ref().or(crl.primary.as_ref()));
        }
    } else {
        if let Some(rl) = payload.rate_limit {
            add_window(rl.primary_window.as_ref().or(rl.primary.as_ref()));
            add_window(rl.secondary_window.as_ref().or(rl.secondary.as_ref()));
        }
        if let Some(crl) = payload.code_review_rate_limit {
            add_window(crl.primary_window.as_ref().or(crl.primary.as_ref()));
        }
    }

    let status_line = format!("{} · {}", plan_type, crate::t("Sesión activa", "Active session"));

    CodexData {
        is_available: true,
        status_line,
        plan_type,
        periods,
        error: None,
    }
}
