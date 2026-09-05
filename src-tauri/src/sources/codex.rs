use crate::http::{get_json, headers_from, post_form};
use crate::i18n::t;
use crate::sources::{as_f64, slug};
use crate::types::{Account, Organization, Profile};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::PathBuf;

const USAGE_URL: &str = "https://chatgpt.com/backend-api/wham/usage";
const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";

fn auth_path() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")).join(".codex").join("auth.json")
}

fn auth() -> Option<Map<String, Value>> {
    let text = fs::read_to_string(auth_path()).ok()?;
    serde_json::from_str::<Value>(&text).ok()?.as_object().cloned()
}

pub fn token() -> Option<String> {
    let a = auth()?;
    let tokens = a.get("tokens").and_then(Value::as_object);
    tokens
        .and_then(|t| t.get("access_token"))
        .or_else(|| a.get("access_token"))
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

pub fn usage() -> Map<String, Value> {
    let Some(tok) = token() else { return err(&t("no_token")) };
    let mut data = request(&tok);
    if data.get("auth_error").and_then(Value::as_bool) == Some(true) {
        if let Some(refreshed) = refresh() {
            data = request(&refreshed);
        }
    }
    if data.contains_key("error") {
        return data;
    }
    normalize(&data)
}

fn request(tok: &str) -> Map<String, Value> {
    let a = auth().unwrap_or_default();
    let tokens = a.get("tokens").and_then(Value::as_object);
    let account_id = tokens.and_then(|t| t.get("account_id")).and_then(Value::as_str).unwrap_or("");
    let mut pairs = vec![
        ("Authorization", format!("Bearer {tok}")),
        ("Accept", "application/json".into()),
        ("User-Agent", "codex_cli_rs/0.136.0".into()),
        ("originator", "codex_cli_rs".into()),
    ];
    if !account_id.is_empty() {
        pairs.push(("ChatGPT-Account-ID", account_id.into()));
    }
    let refs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    get_json(USAGE_URL, &headers_from(&refs), 12)
}

fn refresh() -> Option<String> {
    let mut a = auth()?;
    let tokens = a.get_mut("tokens")?.as_object_mut()?;
    let refresh = tokens.get("refresh_token")?.as_str()?.to_string();
    let headers = headers_from(&[("Content-Type", "application/x-www-form-urlencoded")]);
    let payload = post_form(
        TOKEN_URL,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh),
            ("client_id", CLIENT_ID),
            ("scope", "openid profile email offline_access"),
        ],
        &headers,
    );
    let access = payload.get("access_token")?.as_str()?.to_string();
    tokens.insert("access_token".into(), json!(access.clone()));
    if let Some(r) = payload.get("refresh_token") { tokens.insert("refresh_token".into(), r.clone()); }
    if let Some(r) = payload.get("id_token") { tokens.insert("id_token".into(), r.clone()); }
    let _ = fs::write(auth_path(), serde_json::to_string_pretty(&Value::Object(a)).unwrap_or_default() + "\n");
    Some(access)
}

pub fn profile() -> Option<Profile> {
    let a = auth()?;
    let tokens = a.get("tokens").and_then(Value::as_object);
    let mut email = String::new();
    if let Some(id_token) = tokens.and_then(|t| t.get("id_token")).and_then(Value::as_str) {
        if let Some(claims) = decode_jwt(id_token) {
            email = claims.get("email").and_then(Value::as_str).unwrap_or("").into();
        }
    }
    Some(Profile {
        account: Account {
            email,
            uuid: tokens.and_then(|t| t.get("account_id")).and_then(Value::as_str).unwrap_or("codex").into(),
        },
        organization: Organization { organization_type: "Codex".into() },
    })
}

fn decode_jwt(token: &str) -> Option<Map<String, Value>> {
    let payload = token.split('.').nth(1)?;
    let mut b64 = payload.replace('-', "+").replace('_', "/");
    while b64.len() % 4 != 0 { b64.push('='); }
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn normalize(data: &Map<String, Value>) -> Map<String, Value> {
    let mut quotas = Map::new();
    let mut rate = data.get("rate_limit").or_else(|| data.get("rate_limits")).cloned().unwrap_or(json!({}));
    if let Some(inner) = rate.get("rate_limit").cloned() {
        if inner.is_object() { rate = inner; }
    }
    add_windows(&mut quotas, &rate, "");
    if let Some(extras) = data.get("rate_limits_by_limit_id").and_then(Value::as_object) {
        for (key, value) in extras {
            let prefix = if key.to_lowercase().contains("review") {
                "review".into()
            } else if key.to_lowercase().contains("spark") {
                "spark".into()
            } else {
                slug(key)
            };
            add_windows(&mut quotas, value, &prefix);
        }
    }
    quotas
}

fn add_windows(quotas: &mut Map<String, Value>, snapshot: &Value, prefix: &str) {
    let body = snapshot.get("rate_limit").filter(|v| v.is_object()).unwrap_or(snapshot);
    if let Some(primary) = body.get("primary_window").or_else(|| body.get("primary")).and_then(Value::as_object) {
        let key = if prefix.is_empty() { "five_hour".into() } else { format!("five_hour_{prefix}") };
        quotas.insert(key, window(primary));
    }
    if let Some(secondary) = body.get("secondary_window").or_else(|| body.get("secondary")).and_then(Value::as_object) {
        let key = if prefix.is_empty() { "seven_day".into() } else { format!("seven_day_{prefix}") };
        quotas.insert(key, window(secondary));
    }
}

fn window(w: &Map<String, Value>) -> Value {
    let used = as_f64(w.get("used_percent").or_else(|| w.get("percent_used")));
    let reset = w.get("reset_at").or_else(|| w.get("resets_at")).or_else(|| w.get("resetAt")).and_then(Value::as_str).unwrap_or("");
    json!({ "utilization": used.clamp(0.0, 100.0), "resets_at": reset })
}

fn err(msg: &str) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("error".into(), json!(msg));
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows() {
        let data = json!({"rate_limit":{"primary_window":{"used_percent":12},"secondary_window":{"used_percent":34}}}).as_object().cloned().unwrap();
        let n = normalize(&data);
        assert_eq!(n["five_hour"]["utilization"], 12.0);
        assert_eq!(n["seven_day"]["utilization"], 34.0);
    }
}
