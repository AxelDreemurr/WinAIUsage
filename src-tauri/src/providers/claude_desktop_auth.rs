//! Borrows the OAuth access token of a Claude Desktop login, for users who run
//! Claude Code from Desktop instead of the CLI (whose `~/.claude/.credentials.json`
//! then stays empty).
//!
//! Desktop keeps its token cache in its data folder's `config.json` (see
//! `find_desktop_dir`) under `oauth:tokenCacheV2` (legacy `oauth:tokenCache`), encrypted with Electron's
//! safeStorage: on Windows that is AES-256-GCM (`"v10"` + 12-byte nonce +
//! ciphertext + 16-byte tag) under a key stored DPAPI-protected in `Local State`.
//!
//! Only a currently valid *access* token is read. The refresh token is never
//! used: Anthropic rotates refresh tokens, so refreshing here would sign Desktop
//! out. When the token expires we wait for Desktop to renew it.

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use base64::Engine;
use serde_json::Value;
use std::path::PathBuf;

const API_HOST: &str = "https://api.anthropic.com";
const USAGE_SCOPE: &str = "user:profile";
const INFERENCE_SCOPE: &str = "user:inference";
/// Client id Claude Code / Desktop mint full-scope tokens under.
const PRODUCTION_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
/// Treat tokens expiring within this window as already expired.
const EXPIRY_MARGIN_MS: f64 = 2.0 * 60.0 * 1000.0;

#[derive(Debug, PartialEq)]
pub enum DesktopAuthError {
    /// No Desktop install or no login in its cache.
    NotFound,
    /// Desktop has a login, but its access token expired; Desktop renews it when opened.
    Stale,
    /// The cache exists but couldn't be decrypted or parsed.
    Invalid,
}

#[derive(Debug, PartialEq)]
pub struct DesktopToken {
    pub access_token: String,
    pub subscription_type: Option<String>,
    pub rate_limit_tier: Option<String>,
}

/// `organization` / `account`: the active Claude Code org and user (from `~/.claude.json`),
/// used to pick the right entry when the cache holds several logins.
pub fn read_token(
    organization: Option<&str>,
    account: Option<&str>,
) -> Result<DesktopToken, DesktopAuthError> {
    let dir = find_desktop_dir().ok_or(DesktopAuthError::NotFound)?;

    let config: Value = std::fs::read_to_string(dir.join("config.json"))
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .ok_or(DesktopAuthError::NotFound)?;
    let v2 = config["oauth:tokenCacheV2"].as_str();
    let v1 = config["oauth:tokenCache"].as_str();
    if v2.is_none() && v1.is_none() {
        return Err(DesktopAuthError::NotFound);
    }

    let key = read_master_key(&dir).ok_or(DesktopAuthError::Invalid)?;
    let decode = |stored: Option<&str>| -> Result<Option<Value>, DesktopAuthError> {
        stored
            .map(|s| {
                decrypt_value(&key, s)
                    .and_then(|plain| serde_json::from_slice(&plain).ok())
                    .ok_or(DesktopAuthError::Invalid)
            })
            .transpose()
    };
    let (v2, v1) = (decode(v2)?, decode(v1)?);

    let account = config["lastKnownAccountUuid"].as_str().or(account);
    let now_ms = chrono::Utc::now().timestamp_millis() as f64;
    select_token(v2.as_ref(), v1.as_ref(), organization, account, now_ms)
}

/// Desktop's data folder. The Microsoft Store (MSIX) build writes `%APPDATA%\Claude`
/// virtualized into its package folder, visible at the real path only from inside
/// that package, so other apps must look under `%LOCALAPPDATA%\Packages` too.
fn find_desktop_dir() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(appdata) = std::env::var("APPDATA") {
        candidates.push(PathBuf::from(appdata).join("Claude"));
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        if let Ok(packages) = std::fs::read_dir(PathBuf::from(local).join("Packages")) {
            candidates.extend(
                packages
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_name().to_string_lossy().starts_with("Claude_"))
                    .map(|e| e.path().join("LocalCache").join("Roaming").join("Claude")),
            );
        }
    }
    candidates
        .into_iter()
        .find(|dir| dir.join("config.json").is_file())
}

// ── Decryption ────────────────────────────────────────────────────────────────

/// AES key from `Local State` (`os_crypt.encrypted_key`: "DPAPI" + DPAPI blob).
fn read_master_key(dir: &std::path::Path) -> Option<Vec<u8>> {
    let local_state: Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("Local State")).ok()?).ok()?;
    let wrapped = base64::engine::general_purpose::STANDARD
        .decode(local_state["os_crypt"]["encrypted_key"].as_str()?)
        .ok()?;
    let key = dpapi_unprotect(wrapped.strip_prefix(b"DPAPI")?)?;
    (key.len() == 32).then_some(key)
}

#[cfg(windows)]
fn dpapi_unprotect(data: &[u8]) -> Option<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};

    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len() as u32,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: `input` points to `data`, which outlives the call; on success Windows
    // allocates `output`, which we copy and then release with LocalFree.
    unsafe {
        let ok = CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            &mut output,
        );
        if ok == 0 || output.pbData.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        LocalFree(output.pbData as _);
        Some(bytes)
    }
}

#[cfg(not(windows))]
fn dpapi_unprotect(_data: &[u8]) -> Option<Vec<u8>> {
    None
}

/// Decrypts one base64 safeStorage value (`"v10"` + nonce + ciphertext + tag).
fn decrypt_value(key: &[u8], stored: &str) -> Option<Vec<u8>> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(stored)
        .ok()?;
    let payload = bytes.strip_prefix(b"v10")?;
    if payload.len() < 12 + 16 {
        return None;
    }
    let (nonce, ciphertext) = payload.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(key).ok()?;
    cipher.decrypt(Nonce::from_slice(nonce), ciphertext).ok()
}

// ── Cache selection ───────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    client_id: String,
    organization: String,
    scopes: Vec<String>,
}

/// Keys look like `<client uuid>:<org uuid>:https://api.anthropic.com:<scope> <scope>...`,
/// optionally prefixed with `acct:<user uuid>|` (Desktop migrates legacy keys on use).
fn parse_key(raw: &str) -> Option<CacheKey> {
    let marker = format!(":{API_HOST}:");
    let (prefix, scopes) = raw.split_once(&marker)?;
    let (client_id, organization) = prefix.split_once(':')?;
    if !is_uuid(client_id) || !is_uuid(organization) {
        return None;
    }
    Some(CacheKey {
        client_id: client_id.to_lowercase(),
        organization: organization.to_lowercase(),
        scopes: scopes.split_whitespace().map(String::from).collect(),
    })
}

fn is_uuid(s: &str) -> bool {
    s.len() == 36
        && s.chars().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => c == '-',
            _ => c.is_ascii_hexdigit(),
        })
}

/// Account-scoped entries supersede their legacy alias; other accounts' entries are ignored.
fn normalized(cache: Option<&Value>, account: Option<&str>) -> Vec<(CacheKey, Value)> {
    let Some(map) = cache.and_then(Value::as_object) else {
        return vec![];
    };
    let mut legacy = Vec::new();
    let mut scoped = Vec::new();
    for (raw, entry) in map {
        if let Some(rest) = raw.strip_prefix("acct:") {
            let Some((owner, key)) = rest.split_once('|') else {
                continue;
            };
            if account.is_none_or(|a| !owner.eq_ignore_ascii_case(a)) {
                continue;
            }
            if let Some(k) = parse_key(key) {
                scoped.push((k, entry.clone()));
            }
        } else if let Some(k) = parse_key(raw) {
            legacy.push((k, entry.clone()));
        }
    }
    legacy.retain(|(k, _)| !scoped.iter().any(|(s, _)| s == k));
    legacy.extend(scoped);
    legacy
}

fn select_token(
    v2: Option<&Value>,
    v1: Option<&Value>,
    organization: Option<&str>,
    account: Option<&str>,
    now_ms: f64,
) -> Result<DesktopToken, DesktopAuthError> {
    let v2_entries = normalized(v2, account);
    let v1_entries: Vec<_> = normalized(v1, account)
        .into_iter()
        .filter(|(k, _)| !v2_entries.iter().any(|(k2, _)| k2 == k))
        .collect();

    let mut saw_stale = false;
    let mut saw_invalid = false;
    for entries in [&v2_entries, &v1_entries] {
        let mut best: Option<((u8, u8, usize, f64), DesktopToken)> = None;
        for (key, entry) in entries {
            if organization.is_some_and(|o| !key.organization.eq_ignore_ascii_case(o))
                || !key.scopes.iter().any(|s| s == USAGE_SCOPE)
                || entry.is_null()
            {
                continue;
            }
            let token = entry["token"]
                .as_str()
                .map(str::trim)
                .filter(|t| !t.is_empty());
            let (Some(token), Some(expires_at)) = (token, entry["expiresAt"].as_f64()) else {
                saw_invalid = true;
                continue;
            };
            if expires_at <= now_ms + EXPIRY_MARGIN_MS {
                saw_stale = true;
                continue;
            }
            // Mirror Desktop's own resolution: production client with full scopes first,
            // then any full-scope login, then scope richness; expiry only breaks ties.
            let full = key.scopes.iter().any(|s| s == INFERENCE_SCOPE);
            let rank = (
                u8::from(full && key.client_id == PRODUCTION_CLIENT_ID),
                u8::from(full),
                key.scopes.len(),
                expires_at,
            );
            if best.as_ref().is_none_or(|(r, _)| rank > *r) {
                best = Some((
                    rank,
                    DesktopToken {
                        access_token: token.to_string(),
                        subscription_type: entry["subscriptionType"].as_str().map(String::from),
                        rate_limit_tier: entry["rateLimitTier"].as_str().map(String::from),
                    },
                ));
            }
        }
        if let Some((_, token)) = best {
            return Ok(token);
        }
    }

    Err(if saw_stale {
        DesktopAuthError::Stale
    } else if saw_invalid {
        DesktopAuthError::Invalid
    } else {
        DesktopAuthError::NotFound
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ORG: &str = "17109cf3-e044-4ff7-a095-a90f7340f831";
    const OTHER_ORG: &str = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
    const USER: &str = "11111111-2222-3333-4444-555555555555";
    const OLD_CLIENT: &str = "00000000-0000-0000-0000-000000000000";
    const NOW: f64 = 1_000_000_000_000.0;
    const LATER: f64 = NOW + 3_600_000.0;

    fn key(client: &str, org: &str, scopes: &str) -> String {
        format!("{client}:{org}:{API_HOST}:{scopes}")
    }

    fn entry(token: &str, expires: f64) -> Value {
        json!({ "token": token, "expiresAt": expires, "subscriptionType": "pro" })
    }

    #[test]
    fn decrypts_safe_storage_values() {
        let key = [7u8; 32];
        let nonce = [9u8; 12];
        let cipher = Aes256Gcm::new_from_slice(&key).unwrap();
        let mut stored = b"v10".to_vec();
        stored.extend_from_slice(&nonce);
        stored.extend(
            cipher
                .encrypt(Nonce::from_slice(&nonce), &b"{\"a\":1}"[..])
                .unwrap(),
        );
        let b64 = base64::engine::general_purpose::STANDARD.encode(&stored);

        assert_eq!(decrypt_value(&key, &b64).unwrap(), b"{\"a\":1}");
        assert_eq!(decrypt_value(&[8u8; 32], &b64), None, "wrong key");
        assert_eq!(decrypt_value(&key, "djEw"), None, "truncated");
    }

    #[test]
    fn prefers_production_full_scope_over_longer_lived_leftovers() {
        let cache = json!({
            key(OLD_CLIENT, ORG, USAGE_SCOPE): entry("leftover", LATER + 9e9),
            key(PRODUCTION_CLIENT_ID, ORG, "user:inference user:profile"): entry("current", LATER),
        });
        let t = select_token(Some(&cache), None, Some(ORG), None, NOW).unwrap();
        assert_eq!(t.access_token, "current");
        assert_eq!(t.subscription_type.as_deref(), Some("pro"));
    }

    #[test]
    fn filters_by_organization_and_account() {
        let cache = json!({
            key(PRODUCTION_CLIENT_ID, OTHER_ORG, "user:inference user:profile"): entry("other-org", LATER),
            format!("acct:{OTHER_ORG}|{}", key(PRODUCTION_CLIENT_ID, ORG, "user:inference user:profile")): entry("other-user", LATER),
            format!("acct:{USER}|{}", key(PRODUCTION_CLIENT_ID, ORG, "user:profile")): entry("mine", LATER),
        });
        let t = select_token(Some(&cache), None, Some(ORG), Some(USER), NOW).unwrap();
        assert_eq!(t.access_token, "mine");
    }

    #[test]
    fn reports_stale_and_falls_back_to_v1() {
        let v2 = json!({ key(PRODUCTION_CLIENT_ID, ORG, "user:inference user:profile"): entry("expired", NOW) });
        assert_eq!(
            select_token(Some(&v2), None, Some(ORG), None, NOW),
            Err(DesktopAuthError::Stale)
        );

        let v1 = json!({ key(OLD_CLIENT, ORG, "user:profile"): entry("legacy", LATER) });
        let t = select_token(Some(&json!({})), Some(&v1), Some(ORG), None, NOW).unwrap();
        assert_eq!(t.access_token, "legacy");
        assert_eq!(
            select_token(None, None, Some(ORG), None, NOW),
            Err(DesktopAuthError::NotFound)
        );
    }
}
