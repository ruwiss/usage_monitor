use crate::http::{get_json, headers_from};
use crate::i18n::t;
use crate::types::{Account, Organization, Profile};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::PathBuf;

const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";
const PROFILE_URL: &str = "https://api.anthropic.com/api/oauth/profile";
const FALLBACK_UA: &str = "claude-code/2.1.204";

fn config_dir() -> PathBuf {
    if let Ok(custom) = std::env::var("CLAUDE_CONFIG_DIR") {
        if !custom.is_empty() {
            return PathBuf::from(custom);
        }
    }
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".claude")
}

pub fn token() -> Option<String> {
    let path = config_dir().join(".credentials.json");
    let text = fs::read_to_string(path).ok()?;
    let creds: Value = serde_json::from_str(&text).ok()?;
    creds.get("claudeAiOauth")?.get("accessToken")?.as_str().map(|s| s.to_string()).filter(|s| !s.is_empty())
}

fn headers() -> Option<reqwest::header::HeaderMap> {
    let tok = token()?;
    let version = crate::claude_cli::cli_version(&crate::claude_cli::claude_path());
    let ua = if version.is_empty() { FALLBACK_UA.to_string() } else { format!("claude-code/{version}") };
    Some(headers_from(&[
        ("Authorization", &format!("Bearer {tok}")),
        ("Content-Type", "application/json"),
        ("User-Agent", &ua),
        ("anthropic-beta", "oauth-2025-04-20"),
    ]))
}

pub fn usage() -> Map<String, Value> {
    let Some(h) = headers() else { return error(&t("no_token")) };
    let data = get_json(USAGE_URL, &h, 12);
    if data.contains_key("error") {
        return data;
    }
    merge_limits(data)
}

pub fn profile() -> Option<Profile> {
    let h = headers()?;
    let data = get_json(PROFILE_URL, &h, 12);
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

fn merge_limits(data: Map<String, Value>) -> Map<String, Value> {
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
