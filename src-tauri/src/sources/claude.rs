use crate::http::{get_json, headers_from, post_bytes};
use crate::i18n::t;
use crate::types::{Account, Organization, Profile};
use parking_lot::Mutex;
use serde_json::{json, Map, Value};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
const FALLBACK_UA: &str = "claude-code/2.1.204";
/// Same token endpoint, client and request shape Claude Code uses for its own refresh.
const TOKEN_URL: &str = "https://platform.claude.com/v1/oauth/token";
const CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
const DEFAULT_SCOPES: &str = "user:profile user:inference user:sessions:claude_code user:mcp_servers";
/// Refresh slightly before the stored expiry so a request never races it.
const EXPIRY_MARGIN_MS: i64 = 60_000;

/// Serializes refreshes inside this process (usage and profile can ask at once).
static REFRESH_LOCK: Mutex<()> = parking_lot::const_mutex(());

fn config_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("CLAUDE_CONFIG_DIR") {
        if !custom.is_empty() {
            return PathBuf::from(custom);
        }
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".claude")
}

fn credentials_path() -> PathBuf {
    config_dir().join(".credentials.json")
}

fn read_credentials() -> Option<Map<String, Value>> {
    let text = fs::read_to_string(credentials_path()).ok()?;
    serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')).ok()?.as_object().cloned()
}

fn oauth_of(creds: &Map<String, Value>) -> Option<&Map<String, Value>> {
    creds.get("claudeAiOauth")?.as_object()
}

fn str_field(oauth: &Map<String, Value>, key: &str) -> Option<String> {
    oauth.get(key).and_then(Value::as_str).map(str::to_string).filter(|s| !s.is_empty())
}

pub fn token() -> Option<String> {
    str_field(oauth_of(&read_credentials()?)?, "accessToken")
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// True when the stored access token is past (or about to pass) its `expiresAt`.
/// A missing expiry counts as valid; the server answer decides then.
fn expired(oauth: &Map<String, Value>, now: i64) -> bool {
    oauth.get("expiresAt").and_then(Value::as_i64).is_some_and(|exp| now + EXPIRY_MARGIN_MS >= exp)
}

/// The stored access token, refreshed first when its expiry has passed. Claude Code
/// only refreshes the file while it runs against this config directory, so an idle
/// account would otherwise show "session expired" every ~8 hours.
fn valid_token() -> Option<String> {
    let creds = read_credentials()?;
    let oauth = oauth_of(&creds)?;
    let tok = str_field(oauth, "accessToken")?;
    if expired(oauth, now_ms()) {
        return refresh(&tok).or(Some(tok));
    }
    Some(tok)
}

/// Exchange the stored refresh token for a new access token and write it back the way
/// Claude Code does, so the CLI keeps working with the rotated refresh token.
/// `stale` is the access token that was rejected or expired; when the file already
/// holds a different, unexpired one (another process refreshed), that one is used.
fn refresh(stale: &str) -> Option<String> {
    let _guard = REFRESH_LOCK.lock();
    let creds = read_credentials()?;
    let oauth = oauth_of(&creds)?;
    let current = str_field(oauth, "accessToken");
    if let Some(cur) = &current {
        if cur != stale && !expired(oauth, now_ms()) {
            return Some(cur.clone());
        }
    }
    let refresh_token = str_field(oauth, "refreshToken")?;
    let scope = oauth
        .get("scopes")
        .and_then(Value::as_array)
        .map(|s| s.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_SCOPES.to_string());
    let body = json!({
        "grant_type": "refresh_token",
        "refresh_token": refresh_token,
        "client_id": CLIENT_ID,
        "scope": scope,
    });
    let bytes = post_bytes(
        TOKEN_URL,
        &headers_from(&[("Content-Type", "application/json"), ("Accept", "application/json")]),
        serde_json::to_vec(&body).ok()?,
    )
    .ok()?;
    let payload: Map<String, Value> = serde_json::from_slice::<Value>(&bytes).ok()?.as_object().cloned()?;
    let access = payload.get("access_token").and_then(Value::as_str).filter(|s| !s.is_empty())?.to_string();

    // Re-read before writing: if Claude Code rotated the tokens meanwhile, keep its
    // file untouched and only use our access token for this request.
    if let Some(mut latest) = read_credentials() {
        let unchanged = oauth_of(&latest).and_then(|o| str_field(o, "refreshToken")).as_deref() == Some(refresh_token.as_str());
        if unchanged {
            if let Some(o) = latest.get_mut("claudeAiOauth").and_then(Value::as_object_mut) {
                apply_refresh(o, &payload, &access, now_ms());
                let _ = write_atomic(&credentials_path(), &serde_json::to_string(&Value::Object(latest)).unwrap_or_default());
            }
        }
    }
    Some(access)
}

fn apply_refresh(oauth: &mut Map<String, Value>, payload: &Map<String, Value>, access: &str, now: i64) {
    oauth.insert("accessToken".into(), json!(access));
    if let Some(r) = payload.get("refresh_token").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        oauth.insert("refreshToken".into(), json!(r));
    }
    if let Some(secs) = payload.get("expires_in").and_then(Value::as_i64) {
        oauth.insert("expiresAt".into(), json!(now.saturating_add(secs.saturating_mul(1000))));
    }
    if let Some(secs) = payload.get("refresh_token_expires_in").and_then(Value::as_i64) {
        oauth.insert("refreshTokenExpiresAt".into(), json!(now.saturating_add(secs.saturating_mul(1000))));
    }
    if let Some(scope) = payload.get("scope").and_then(Value::as_str).filter(|s| !s.trim().is_empty()) {
        oauth.insert("scopes".into(), json!(scope.split_whitespace().collect::<Vec<_>>()));
    }
}

/// Write via a sibling temp file and rename, so Claude Code never reads a torn file.
fn write_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    let tmp = path.with_extension("json.usage-monitor.tmp");
    {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
    }
    fs::rename(&tmp, path).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

fn headers(tok: &str) -> reqwest::header::HeaderMap {
    // Spawning the CLI per request is slow; the version only changes on upgrade.
    static VERSION: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let version = VERSION.get_or_init(|| crate::claude_cli::cli_version(&crate::claude_cli::claude_path()));
    let ua = if version.is_empty() { FALLBACK_UA.to_string() } else { format!("claude-code/{version}") };
    headers_from(&[
        ("Authorization", &format!("Bearer {tok}")),
        ("Content-Type", "application/json"),
        ("User-Agent", &ua),
        ("anthropic-beta", "oauth-2025-04-20"),
    ])
}

/// GET with the current token; on 401 refresh once and retry.
fn authed_get(url: &str) -> Option<Map<String, Value>> {
    let tok = valid_token()?;
    let data = get_json(url, &headers(&tok), 12);
    if data.get("auth_error").and_then(Value::as_bool) == Some(true) {
        if let Some(fresh) = refresh(&tok).filter(|f| *f != tok) {
            return Some(get_json(url, &headers(&fresh), 12));
        }
    }
    Some(data)
}

pub fn usage() -> Map<String, Value> {
    let Some(data) = authed_get(USAGE_URL) else { return error(&t("no_token")) };
    if data.contains_key("error") {
        return data;
    }
    merge_limits(data)
}

pub fn profile() -> Option<Profile> {
    let data = authed_get(PROFILE_URL)?;
    if data.contains_key("error") {
        return None;
    }
    Some(profile_from(&data))
}

fn profile_from(data: &Map<String, Value>) -> Profile {
    let account = data.get("account").and_then(Value::as_object);
    let org = data.get("organization").and_then(Value::as_object);
    Profile {
        account: Account {
            email: account.and_then(|a| a.get("email")).and_then(Value::as_str).unwrap_or("").into(),
            uuid: account.and_then(|a| a.get("uuid")).and_then(Value::as_str).unwrap_or("").into(),
        },
        organization: Organization {
            organization_type: org.and_then(|o| o.get("organization_type")).and_then(Value::as_str).unwrap_or("").into(),
        },
    }
}

pub(crate) fn merge_limits(data: Map<String, Value>) -> Map<String, Value> {
    let Some(limits) = data.get("limits").and_then(Value::as_array).cloned() else { return data };
    let mut reset_to_field = Map::new();
    for (key, value) in &data {
        if let Some(obj) = value.as_object() {
            if obj.get("utilization").is_some() {
                if let Some(r) = obj.get("resets_at").and_then(Value::as_str) {
                    if !r.is_empty() {
                        reset_to_field.entry(r.to_string()).or_insert(json!(key));
                    }
                }
            }
        }
    }
    let mut group_prefix = Map::new();
    for limit in &limits {
        let Some(obj) = limit.as_object() else { continue };
        if obj.get("scope").is_some() { continue; }
        if let (Some(group), Some(resets)) = (obj.get("group").and_then(Value::as_str), obj.get("resets_at").and_then(Value::as_str)) {
            if reset_to_field.contains_key(resets) {
                group_prefix.entry(group.to_string()).or_insert(reset_to_field[resets].clone());
            }
        }
    }
    let mut merged = data.clone();
    for limit in &limits {
        let Some(obj) = limit.as_object() else { continue };
        let display = obj.get("scope").and_then(|s| s.get("model")).and_then(|m| m.get("display_name")).and_then(Value::as_str);
        let prefix = obj.get("group").and_then(Value::as_str).and_then(|g| group_prefix.get(g)).and_then(Value::as_str);
        let (Some(display), Some(prefix)) = (display, prefix) else { continue };
        let field = format!("{prefix}_{}", crate::sources::slug(display));
        if merged.get(&field).map(|v| !v.is_null()).unwrap_or(false) { continue; }
        merged.insert(field, json!({
            "utilization": obj.get("percent").and_then(Value::as_f64).unwrap_or(0.0),
            "resets_at": obj.get("resets_at"),
        }));
    }
    merged
}

fn error(msg: &str) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("error".into(), json!(msg));
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oauth(v: Value) -> Map<String, Value> {
        v.as_object().cloned().unwrap()
    }

    #[test]
    fn expiry_uses_margin_and_tolerates_missing_field() {
        let o = oauth(json!({ "expiresAt": 1_000_000 }));
        assert!(!expired(&o, 1_000_000 - EXPIRY_MARGIN_MS - 1));
        assert!(expired(&o, 1_000_000 - EXPIRY_MARGIN_MS));
        assert!(expired(&o, 2_000_000));
        assert!(!expired(&oauth(json!({})), i64::MAX / 2));
    }

    #[test]
    fn refresh_payload_updates_only_token_fields() {
        let mut o = oauth(json!({
            "accessToken": "old", "refreshToken": "r1", "expiresAt": 1,
            "scopes": ["user:inference"], "subscriptionType": "max"
        }));
        let payload = oauth(json!({
            "access_token": "new", "refresh_token": "r2", "expires_in": 28800,
            "refresh_token_expires_in": 100, "scope": "user:inference user:profile"
        }));
        apply_refresh(&mut o, &payload, "new", 10_000);
        assert_eq!(o["accessToken"], "new");
        assert_eq!(o["refreshToken"], "r2");
        assert_eq!(o["expiresAt"], 10_000 + 28_800_000);
        assert_eq!(o["refreshTokenExpiresAt"], 10_000 + 100_000);
        assert_eq!(o["scopes"], json!(["user:inference", "user:profile"]));
        assert_eq!(o["subscriptionType"], "max");
    }

    #[test]
    fn refresh_without_rotation_keeps_refresh_token() {
        let mut o = oauth(json!({ "accessToken": "old", "refreshToken": "r1" }));
        apply_refresh(&mut o, &oauth(json!({ "access_token": "new" })), "new", 0);
        assert_eq!(o["refreshToken"], "r1");
        assert!(o.get("expiresAt").is_none());
    }
}
