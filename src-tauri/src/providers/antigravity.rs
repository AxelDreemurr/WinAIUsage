use crate::pricing::{self, DailyUsage, TokenUsage};
use serde::Serialize;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::time::Duration;

const CREATE_NO_WINDOW: u32 = 0x08000000;

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
    /// Today's tokens and API-equivalent cost, from the local conversation databases.
    pub today: Option<DailyUsage>,
    pub error: Option<String>,
}

impl AntigravityData {
    pub fn loading() -> Self {
        Self {
            is_available: false,
            plan_name: String::new(),
            models: vec![],
            status_line: crate::t("Cargando...", "Loading...").to_string(),
            today: None,
            error: None,
        }
    }

    fn not_running() -> Self {
        Self {
            is_available: false,
            plan_name: String::new(),
            models: vec![],
            status_line: crate::t(
                "Abre Antigravity para monitorear su uso",
                "Open Antigravity to monitor its usage",
            )
            .to_string(),
            today: None,
            error: None,
        }
    }

    fn unavailable(error: impl Into<String>) -> Self {
        Self {
            is_available: false,
            plan_name: String::new(),
            models: vec![],
            status_line: String::new(),
            today: None,
            error: Some(error.into()),
        }
    }

    fn from_models(plan: impl Into<String>, models: Vec<ModelQuota>) -> Self {
        let plan = plan.into();
        let n = models.iter().filter(|m| m.remaining_fraction > 0.0).count();
        let status_line = if crate::is_es() {
            format!("{} · {} modelos disponibles", plan, n)
        } else {
            format!("{} · {} models available", plan, n)
        };
        Self {
            is_available: true,
            plan_name: plan,
            models,
            status_line,
            today: None,
            error: None,
        }
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn get_data() -> AntigravityData {
    // Local databases don't depend on the language server, so today's usage shows even when it's closed.
    let today = tokio::task::spawn_blocking(|| read_usage(pricing::today()))
        .await
        .ok();

    let mut data = fetch_status().await;
    data.today = today;
    data
}

async fn fetch_status() -> AntigravityData {
    if let Some(data) = try_language_server().await {
        return data;
    }
    if let Some(data) = try_cloud_api().await {
        return data;
    }
    AntigravityData::not_running()
}

// ── Strategy 1: Language Server ───────────────────────────────────────────────

struct LsInfo {
    pid: u32,
    csrf_token: String,
    extension_server_port: Option<u16>,
}

async fn try_language_server() -> Option<AntigravityData> {
    let info = find_ls_process()?;

    eprintln!(
        "[antigravity] LS found — PID={} csrf={}... ext_port={:?}",
        info.pid,
        &info.csrf_token[..info.csrf_token.len().min(8)],
        info.extension_server_port
    );

    let ports = find_listening_ports(info.pid);
    eprintln!(
        "[antigravity] Listening ports for PID {}: {:?}",
        info.pid, ports
    );

    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(true)
        .timeout(Duration::from_secs(5))
        .build()
        .ok()?;

    // Paso 3: probe HTTPS on every netstat port
    let working = probe_ports(&client, &ports, "https", &info.csrf_token).await;

    // Fallback: HTTP on --extension_server_port from CommandLine
    let working = if working.is_none() {
        if let Some(ext) = info.extension_server_port {
            eprintln!("[antigravity] HTTPS failed — retrying HTTP on port {}", ext);
            probe_ports(&client, &[ext], "http", &info.csrf_token).await
        } else {
            None
        }
    } else {
        working
    };

    let (scheme, port) = match working {
        Some(p) => p,
        None => {
            eprintln!("[antigravity] No working port found");
            return Some(AntigravityData::unavailable(crate::t(
                "Language server sin puerto disponible",
                "Language server with no port available",
            )));
        }
    };

    eprintln!(
        "[antigravity] Calling GetUserStatus on {}://127.0.0.1:{}",
        scheme, port
    );
    call_get_user_status(&client, &scheme, port, &info.csrf_token).await
}

// ── Paso 1: find process ──────────────────────────────────────────────────────

fn find_ls_process() -> Option<LsInfo> {
    eprintln!("[antigravity] Running wmic to find language_server process...");

    let wmic = std::process::Command::new("wmic")
        .args([
            "process",
            "where",
            "name like '%language_server%'",
            "get",
            "ProcessId,CommandLine",
            "/format:csv",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    if let Ok(out) = wmic {
        let text = decode_wmic(&out.stdout);
        eprintln!(
            "[antigravity] wmic output ({} bytes): {}",
            text.len(),
            &text[..text.len().min(300)]
        );
        if let Some(info) = parse_wmic_csv(&text) {
            return Some(info);
        }
    }

    // PowerShell fallback
    eprintln!("[antigravity] wmic found nothing — trying PowerShell fallback...");
    let ps = std::process::Command::new("powershell")
        .args([
            "-command",
            "Get-WmiObject Win32_Process | Where-Object {$_.Name -like '*language_server*' -and $_.CommandLine -like '*antigravity*'} | Select-Object ProcessId,CommandLine | ConvertTo-Json",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    match ps {
        Ok(out) => {
            let text = String::from_utf8_lossy(&out.stdout).into_owned();
            eprintln!(
                "[antigravity] PowerShell output: {}",
                &text[..text.len().min(300)]
            );
            parse_ps_json(&text)
        }
        Err(e) => {
            eprintln!("[antigravity] PowerShell also failed: {}", e);
            None
        }
    }
}

fn parse_wmic_csv(text: &str) -> Option<LsInfo> {
    for line in text.lines() {
        let lower = line.to_lowercase();
        if !lower.contains("antigravity") {
            continue;
        }
        eprintln!(
            "[antigravity] Matching wmic line: {}",
            &line[..line.len().min(300)]
        );
        let pid = extract_pid(line)?;
        let csrf = extract_arg(line, "--csrf_token").unwrap_or_default();
        let ext_port =
            extract_arg(line, "--extension_server_port").and_then(|s| s.trim().parse().ok());
        return Some(LsInfo {
            pid,
            csrf_token: csrf,
            extension_server_port: ext_port,
        });
    }
    None
}

fn parse_ps_json(text: &str) -> Option<LsInfo> {
    let v: serde_json::Value = serde_json::from_str(text.trim()).ok()?;

    // ConvertTo-Json outputs array when multiple results, object for single
    let item = if v.is_array() {
        v.as_array()?.first()?.clone()
    } else {
        v
    };

    let cmd = item["CommandLine"].as_str().unwrap_or("");
    let pid = item["ProcessId"].as_u64()? as u32;
    let csrf = extract_arg(cmd, "--csrf_token").unwrap_or_default();
    let ext_port = extract_arg(cmd, "--extension_server_port").and_then(|s| s.trim().parse().ok());

    Some(LsInfo {
        pid,
        csrf_token: csrf,
        extension_server_port: ext_port,
    })
}

// ── Paso 2: netstat ports ─────────────────────────────────────────────────────

fn find_listening_ports(pid: u32) -> Vec<u16> {
    let out = match std::process::Command::new("netstat")
        .args(["-ano"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        Ok(o) => o,
        Err(_) => return vec![],
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let pid_str = pid.to_string();

    text.lines()
        .filter(|l| {
            l.contains("LISTENING") && l.split_whitespace().last() == Some(pid_str.as_str())
        })
        .filter_map(|l| {
            // Local address is 2nd token: "0.0.0.0:PORT" or "127.0.0.1:PORT" or "[::]:PORT"
            let local = l.split_whitespace().nth(1)?;
            local.rsplit(':').next()?.parse().ok()
        })
        .collect()
}

// ── Paso 3: probe ports ───────────────────────────────────────────────────────

async fn probe_ports(
    client: &reqwest::Client,
    ports: &[u16],
    scheme: &str,
    csrf: &str,
) -> Option<(String, u16)> {
    for &port in ports {
        let url = format!(
            "{}://127.0.0.1:{}/exa.language_server_pb.LanguageServerService/GetUnleashData",
            scheme, port
        );
        eprintln!("[antigravity] Probing {}", url);
        let ok = client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Connect-Protocol-Version", "1")
            .header("x-codeium-csrf-token", csrf)
            .body("{}")
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false);

        if ok {
            eprintln!("[antigravity] Port {} ({}) OK", port, scheme);
            return Some((scheme.to_string(), port));
        }
    }
    None
}

// ── Paso 4: GetUserStatus ─────────────────────────────────────────────────────

async fn call_get_user_status(
    client: &reqwest::Client,
    scheme: &str,
    port: u16,
    csrf: &str,
) -> Option<AntigravityData> {
    let url = format!(
        "{}://127.0.0.1:{}/exa.language_server_pb.LanguageServerService/GetUserStatus",
        scheme, port
    );
    let body = serde_json::json!({
        "metadata": {
            "ideName": "antigravity",
            "extensionName": "antigravity",
            "ideVersion": "unknown",
            "locale": "en"
        }
    });

    let resp = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Connect-Protocol-Version", "1")
        .header("x-codeium-csrf-token", csrf)
        .json(&body)
        .send()
        .await
        .ok()?;

    if !resp.status().is_success() {
        eprintln!("[antigravity] GetUserStatus returned {}", resp.status());
        return None;
    }

    let v: serde_json::Value = resp.json().await.ok()?;
    eprintln!(
        "[antigravity] GetUserStatus response keys: {:?}",
        v.as_object().map(|m| m.keys().collect::<Vec<_>>())
    );

    let plan = v["userStatus"]["planStatus"]["planInfo"]["planName"]
        .as_str()
        .unwrap_or("Pro")
        .to_string();

    let models = parse_ls_models(&v);
    Some(AntigravityData::from_models(plan, models))
}

// ── Parse helpers ─────────────────────────────────────────────────────────────

fn parse_ls_models(v: &serde_json::Value) -> Vec<ModelQuota> {
    let arr = match v["userStatus"]["cascadeModelConfigData"]["clientModelConfigs"].as_array() {
        Some(a) => a,
        None => return vec![],
    };
    let mut models: Vec<ModelQuota> = arr
        .iter()
        .filter_map(|c| {
            let label = c["label"].as_str()?;
            let remaining = c["quotaInfo"]["remainingFraction"].as_f64().unwrap_or(1.0);
            let reset_time = c["quotaInfo"]["resetTime"]
                .as_str()
                .unwrap_or("")
                .to_string();
            Some(ModelQuota {
                label: label.to_string(),
                remaining_fraction: remaining,
                percent_used: (1.0 - remaining) * 100.0,
                reset_time,
            })
        })
        .collect();
    models.sort_by_key(|m| model_sort_key(&m.label));
    models
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
    if val.is_empty() {
        None
    } else {
        Some(val)
    }
}

fn extract_pid(line: &str) -> Option<u32> {
    // ProcessId = last comma-separated field in WMIC CSV
    let last = line.rfind(',')?;
    line[last + 1..].trim().parse().ok()
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
    let mut models: Vec<ModelQuota> = map
        .values()
        .filter_map(|m| {
            let label = m["displayName"].as_str()?;
            if label.is_empty() {
                return None;
            }
            let remaining = m["quotaInfo"]["remainingFraction"].as_f64().unwrap_or(1.0);
            let reset_time = m["quotaInfo"]["resetTime"]
                .as_str()
                .unwrap_or("")
                .to_string();
            Some(ModelQuota {
                label: label.to_string(),
                remaining_fraction: remaining,
                percent_used: (1.0 - remaining) * 100.0,
                reset_time,
            })
        })
        .collect();
    models.sort_by_key(|m| model_sort_key(&m.label));
    models
}

/// Sort priority: Gemini=0, Claude=1, GPT-OSS=2, everything else=3.
/// Within the same group, the original relative order is preserved (stable sort).
fn model_sort_key(label: &str) -> u8 {
    let lower = label.to_lowercase();
    if lower.contains("gemini") {
        0
    } else if lower.contains("claude") {
        1
    } else if lower.contains("gpt") {
        2
    } else {
        3
    }
}

// ── Local conversation databases ──────────────────────────────────────────────

/// Sums one local day's model generations across Antigravity conversations.
///
/// Each conversation is a SQLite database (`~/.gemini/antigravity*/conversations/*.db`)
/// whose `gen_metadata` table holds one protobuf row per generation:
/// `1.19` model id, `1.9.4.1` creation time (unix seconds), and `1.4` usage stats
/// (`2` uncached input, `3` output incl. thinking, `4` cache write, `5` cache read).
/// Rows from older app versions carry no creation time and are skipped.
fn read_usage(day: chrono::NaiveDate) -> DailyUsage {
    let mut usage = DailyUsage::default();
    let Ok(userprofile) = std::env::var("USERPROFILE") else {
        return usage;
    };
    let gemini_dir = std::path::PathBuf::from(userprofile).join(".gemini");

    let Ok(entries) = std::fs::read_dir(&gemini_dir) else {
        return usage;
    };
    for app_dir in entries.filter_map(|e| e.ok()) {
        if !app_dir
            .file_name()
            .to_string_lossy()
            .starts_with("antigravity")
        {
            continue;
        }
        let Ok(dbs) = std::fs::read_dir(app_dir.path().join("conversations")) else {
            continue;
        };
        for db in dbs.filter_map(|e| e.ok()).map(|e| e.path()) {
            if db.extension().is_none_or(|ext| ext != "db") {
                continue;
            }
            // While the app runs, new rows land in the WAL and the .db mtime may not change.
            let wal = db.with_extension("db-wal");
            if !pricing::modified_since(&db, day) && !pricing::modified_since(&wal, day) {
                continue;
            }
            for row in read_gen_metadata(&db, &wal) {
                let Some(stats) = pb::message(&row, &[1]) else {
                    continue;
                };
                let created = pb::message(stats, &[9, 4]).and_then(|t| pb::varint(t, 1));
                if created.and_then(|t| pricing::local_date_of_unix(t as i64)) != Some(day) {
                    continue;
                }
                let Some(u) = pb::message(stats, &[4]) else {
                    continue;
                };
                let n = |field: u32| pb::varint(u, field).unwrap_or(0);
                let tokens = TokenUsage {
                    input: n(2),
                    output: n(3),
                    cache_write_5m: n(4),
                    cache_write_1h: 0,
                    cache_read: n(5),
                };
                let model = pb::string(stats, 19).unwrap_or("unknown");
                usage.add(model, &tokens, pricing::any(model, tokens.prompt()));
            }
        }
    }

    usage
}

/// Reads `gen_metadata` from a snapshot copy, so the running app's locks and WAL are untouched.
fn read_gen_metadata(db: &std::path::Path, wal: &std::path::Path) -> Vec<Vec<u8>> {
    let dir = std::env::temp_dir().join(format!("winaiusage-ag-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let copy = dir.join("conversation.db");
    let copy_wal = dir.join("conversation.db-wal");
    let _ = std::fs::remove_file(&copy_wal);
    let _ = std::fs::remove_file(dir.join("conversation.db-shm"));
    if std::fs::copy(db, &copy).is_err() {
        return vec![];
    }
    if wal.exists() {
        let _ = std::fs::copy(wal, &copy_wal);
    }

    let rows = (|| -> rusqlite::Result<Vec<Vec<u8>>> {
        let conn = rusqlite::Connection::open(&copy)?;
        let mut stmt = conn.prepare("SELECT data FROM gen_metadata")?;
        let rows = stmt
            .query_map([], |r| r.get::<_, Vec<u8>>(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    })()
    .unwrap_or_default();

    let _ = std::fs::remove_dir_all(&dir);
    rows
}

/// Minimal protobuf reader: just enough to walk known field paths.
mod pb {
    fn read_varint(buf: &[u8], pos: &mut usize) -> Option<u64> {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = *buf.get(*pos)?;
            *pos += 1;
            value |= u64::from(byte & 0x7f) << shift;
            if byte < 0x80 {
                return Some(value);
            }
        }
        None
    }

    enum Value<'a> {
        Varint(u64),
        Bytes(&'a [u8]),
        Fixed,
    }

    /// First occurrence of field `wanted` in `buf`.
    fn field(buf: &[u8], wanted: u32) -> Option<Value<'_>> {
        let mut pos = 0;
        while pos < buf.len() {
            let key = read_varint(buf, &mut pos)?;
            let (num, wire) = ((key >> 3) as u32, key & 7);
            let value = match wire {
                0 => Value::Varint(read_varint(buf, &mut pos)?),
                1 => {
                    pos += 8;
                    Value::Fixed
                }
                2 => {
                    let len = read_varint(buf, &mut pos)? as usize;
                    let bytes = buf.get(pos..pos.checked_add(len)?)?;
                    pos += len;
                    Value::Bytes(bytes)
                }
                5 => {
                    pos += 4;
                    Value::Fixed
                }
                _ => return None,
            };
            if num == wanted {
                return Some(value);
            }
        }
        None
    }

    /// Nested message at `path` (e.g. `[1, 9, 4]`).
    pub fn message<'a>(buf: &'a [u8], path: &[u32]) -> Option<&'a [u8]> {
        path.iter().try_fold(buf, |b, &f| match field(b, f)? {
            Value::Bytes(bytes) => Some(bytes),
            _ => None,
        })
    }

    pub fn varint(buf: &[u8], num: u32) -> Option<u64> {
        match field(buf, num)? {
            Value::Varint(v) => Some(v),
            _ => None,
        }
    }

    pub fn string(buf: &[u8], num: u32) -> Option<&str> {
        match field(buf, num)? {
            Value::Bytes(b) => std::str::from_utf8(b).ok(),
            _ => None,
        }
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn walks_nested_fields() {
            // 1: { 4: { 2: 300 }, 19: "gemini" }
            let inner = [0x10, 0xac, 0x02];
            let mut stats = vec![0x22, inner.len() as u8];
            stats.extend_from_slice(&inner);
            stats.extend_from_slice(&[0x9a, 0x01, 6]);
            stats.extend_from_slice(b"gemini");
            let mut root = vec![0x0a, stats.len() as u8];
            root.extend_from_slice(&stats);

            let s = super::message(&root, &[1]).unwrap();
            assert_eq!(
                super::varint(super::message(s, &[4]).unwrap(), 2),
                Some(300)
            );
            assert_eq!(super::string(s, 19), Some("gemini"));
            assert_eq!(super::message(&root, &[2]), None);
        }
    }
}
