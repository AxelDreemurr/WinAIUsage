use serde::Serialize;
use std::time::Duration;

// ── Public structs ────────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
pub struct ModelQuota {
    pub label: String,
    pub remaining_fraction: f64,
    pub percent_used: f64,
    pub reset_time: String,
}

#[derive(Serialize, Clone)]
pub struct AntigravityData {
    pub is_available: bool,
    pub plan_name: String,
    pub models: Vec<ModelQuota>,
    pub status_line: String,
    pub error: Option<String>,
}

impl AntigravityData {
    pub fn loading() -> Self {
        Self {
            is_available: false,
            plan_name: String::new(),
            models: vec![],
            status_line: "Cargando...".to_string(),
            error: None,
        }
    }

    fn unavailable(error: impl Into<String>) -> Self {
        Self {
            is_available: false,
            plan_name: String::new(),
            models: vec![],
            status_line: String::new(),
            error: Some(error.into()),
        }
    }

    fn from_models(plan: impl Into<String>, models: Vec<ModelQuota>) -> Self {
        let plan = plan.into();
        let n = models.iter().filter(|m| m.remaining_fraction > 0.0).count();
        let status_line = format!("{} · {} modelos disponibles", plan, n);
        Self {
            is_available: true,
            plan_name: plan,
            models,
            status_line,
            error: None,
        }
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn get_data() -> AntigravityData {
    if let Some(data) = try_language_server().await {
        return data;
    }
    if let Some(data) = try_cloud_api().await {
        return data;
    }
    AntigravityData::unavailable("Inicia Antigravity y vuelve a intentar")
}

// ── Strategy 1: Language Server ───────────────────────────────────────────────

async fn try_language_server() -> Option<AntigravityData> {
    let (port, csrf) = find_ls_info()?;

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(5))
        .build()
        .ok()?;

    let base = format!("http://127.0.0.1:{}", port);

    // Verify port with GetUnleashData
    let probe_ok = client
        .post(format!(
            "{}/exa.language_server_pb.LanguageServerService/GetUnleashData",
            base
        ))
        .header("Content-Type", "application/json")
        .header("Connect-Protocol-Version", "1")
        .header("x-codeium-csrf-token", &csrf)
        .body("{}")
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false);

    if !probe_ok {
        return None;
    }

    // GetUserStatus
    let body = serde_json::json!({
        "metadata": {
            "ideName": "antigravity",
            "extensionName": "antigravity",
            "ideVersion": "unknown",
            "locale": "en"
        }
    });

    let resp = client
        .post(format!(
            "{}/exa.language_server_pb.LanguageServerService/GetUserStatus",
            base
        ))
        .header("Content-Type", "application/json")
        .header("Connect-Protocol-Version", "1")
        .header("x-codeium-csrf-token", &csrf)
        .json(&body)
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let v: serde_json::Value = resp.json().await.ok()?;

    let plan = v["userStatus"]["planStatus"]["planInfo"]["planName"]
        .as_str()
        .unwrap_or("Pro")
        .to_string();

    let models = parse_ls_models(&v);
    Some(AntigravityData::from_models(plan, models))
}

fn find_ls_info() -> Option<(u16, String)> {
    let output = std::process::Command::new("wmic")
        .args(["process", "get", "CommandLine,ProcessId", "/format:csv"])
        .output()
        .ok()?;

    let text = decode_wmic(&output.stdout);

    for line in text.lines() {
        let lower = line.to_lowercase();
        if lower.contains("antigravity")
            && (lower.contains("language_server") || lower.contains("codeium"))
        {
            // Try extracting port and csrf from args
            if let (Some(port_s), Some(csrf)) = (
                extract_arg(line, "--extension_server_port"),
                extract_arg(line, "--csrf_token"),
            ) {
                if let Ok(port) = port_s.parse::<u16>() {
                    return Some((port, csrf));
                }
            }

            // Fallback: find listening port via netstat + PID
            if let Some(pid) = extract_pid(line) {
                if let Some(&port) = find_listening_ports(pid).first() {
                    return Some((port, String::new()));
                }
            }
        }
    }
    None
}

fn decode_wmic(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFF, 0xFE]) {
        let utf16: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&utf16)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

fn extract_arg(line: &str, flag: &str) -> Option<String> {
    let needle = format!("{} ", flag);
    let idx = line.find(&needle)?;
    let after = line[idx + needle.len()..].trim_start_matches('"');
    let end = after
        .find(|c: char| c == ' ' || c == '"' || c == '\r' || c == '\n')
        .unwrap_or(after.len());
    let val = after[..end].trim().to_string();
    if val.is_empty() { None } else { Some(val) }
}

fn extract_pid(line: &str) -> Option<u32> {
    // ProcessId is the last comma-separated field in WMIC CSV
    let last = line.rfind(',')?;
    line[last + 1..].trim().parse().ok()
}

fn find_listening_ports(pid: u32) -> Vec<u16> {
    let output = match std::process::Command::new("netstat").args(["-ano"]).output() {
        Ok(o) => o,
        Err(_) => return vec![],
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let pid_str = pid.to_string();

    text.lines()
        .filter(|l| {
            l.contains("LISTENING") && l.split_whitespace().last() == Some(pid_str.as_str())
        })
        .filter_map(|l| {
            let local = l.split_whitespace().nth(1)?;
            local.rsplit(':').next()?.parse().ok()
        })
        .collect()
}

fn parse_ls_models(v: &serde_json::Value) -> Vec<ModelQuota> {
    let arr = match v["userStatus"]["cascadeModelConfigData"]["clientModelConfigs"].as_array() {
        Some(a) => a,
        None => return vec![],
    };
    arr.iter()
        .filter_map(|c| {
            let label = c["label"].as_str()?;
            let remaining = c["quotaInfo"]["remainingFraction"].as_f64().unwrap_or(1.0);
            let reset_time = c["quotaInfo"]["resetTime"].as_str().unwrap_or("").to_string();
            Some(ModelQuota {
                label: label.to_string(),
                remaining_fraction: remaining,
                percent_used: (1.0 - remaining) * 100.0,
                reset_time,
            })
        })
        .collect()
}

// ── Strategy 2: Cloud Code API ────────────────────────────────────────────────

async fn try_cloud_api() -> Option<AntigravityData> {
    let api_key = read_api_key_from_sqlite()?;

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .ok()?;

    let resp = client
        .post("https://cloudcode-pa.googleapis.com/v1internal:fetchAvailableModels")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .header("User-Agent", "antigravity")
        .body("{}")
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        return None;
    }

    let v: serde_json::Value = resp.json().await.ok()?;
    let models = parse_cloud_models(&v);
    if models.is_empty() {
        return None;
    }

    Some(AntigravityData::from_models("Cloud", models))
}

fn read_api_key_from_sqlite() -> Option<String> {
    let appdata = std::env::var("APPDATA").ok()?;
    let db_path = std::path::PathBuf::from(appdata)
        .join("Antigravity")
        .join("User")
        .join("globalStorage")
        .join("state.vscdb");

    let conn = rusqlite::Connection::open(&db_path).ok()?;
    let raw: String = conn
        .query_row(
            "SELECT value FROM ItemTable WHERE key = 'antigravityAuthStatus'",
            [],
            |row| row.get(0),
        )
        .ok()?;

    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    v["apiKey"].as_str().map(|s| s.to_string())
}

fn parse_cloud_models(v: &serde_json::Value) -> Vec<ModelQuota> {
    let map = match v["models"].as_object() {
        Some(m) => m,
        None => return vec![],
    };
    map.values()
        .filter_map(|m| {
            let label = m["displayName"].as_str()?;
            if label.is_empty() {
                return None;
            }
            let remaining = m["quotaInfo"]["remainingFraction"].as_f64().unwrap_or(1.0);
            let reset_time = m["quotaInfo"]["resetTime"].as_str().unwrap_or("").to_string();
            Some(ModelQuota {
                label: label.to_string(),
                remaining_fraction: remaining,
                percent_used: (1.0 - remaining) * 100.0,
                reset_time,
            })
        })
        .collect()
}
