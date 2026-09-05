use crate::http::{get_json, headers_from};
use crate::settings::Settings;
use crate::sources::{normalize_quotas, usage_providers};
use crate::types::{Account, Organization, Profile};
use serde_json::{json, Map, Value};

#[derive(Clone)]
pub struct Conn {
    pub id: String,
    pub provider: String,
    pub name: String,
    pub email: String,
    pub is_active: bool,
}

fn base_url(settings: &Settings) -> String {
    std::env::var("NINEROUTER_URL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| settings.ninerouter_url.clone())
        .trim()
        .trim_end_matches('/')
        .to_string()
}

pub fn connections(settings: &Settings) -> Vec<Conn> {
    let url = format!("{}/api/providers", base_url(settings));
    let data = get_json(&url, &headers_from(&[]), 4);
    let raw = data.get("connections").cloned().unwrap_or(Value::Array(vec![]));
    let Some(list) = raw.as_array() else { return vec![] };
    let mut items = Vec::new();
    for item in list {
        let Some(obj) = item.as_object() else { continue };
        let Some(id) = obj.get("id").and_then(Value::as_str) else { continue };
        let provider = obj.get("provider").and_then(Value::as_str).unwrap_or("").to_string();
        if !usage_providers().contains(provider.as_str()) {
            continue;
        }
        items.push(Conn {
            id: id.into(),
            provider,
            name: obj.get("name").or_else(|| obj.get("email")).and_then(Value::as_str).unwrap_or("").trim().into(),
            email: obj.get("email").and_then(Value::as_str).unwrap_or("").trim().into(),
            is_active: obj.get("isActive").and_then(Value::as_bool).unwrap_or(true),
        });
    }
    items.sort_by(|a, b| {
        (if a.is_active { 0 } else { 1 })
            .cmp(&(if b.is_active { 0 } else { 1 }))
            .then(a.provider.cmp(&b.provider))
            .then(a.name.cmp(&b.name))
    });
    items
}

pub fn usage(settings: &Settings, connection_id: &str) -> Map<String, Value> {
    let url = format!("{}/api/usage/{connection_id}", base_url(settings));
    let mut data = get_json(&url, &headers_from(&[]), 12);
    if data.get("auth_error").and_then(Value::as_bool) == Some(true) {
        data = get_json(&url, &headers_from(&[]), 12);
    }
    if data.contains_key("error") {
        return data;
    }
    if data.get("message").is_some() && data.get("quotas").and_then(Value::as_object).map(|q| q.is_empty()).unwrap_or(true) {
        let mut m = Map::new();
        m.insert("error".into(), data.get("message").cloned().unwrap_or(json!("error")));
        return m;
    }
    let mut result = normalize_quotas(data.get("quotas").and_then(Value::as_object).unwrap_or(&Map::new()));
    if let Some(plan) = data.get("plan").and_then(Value::as_str).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        result.insert("_plan".into(), json!(plan));
    }
    result
}

pub fn profile(settings: &Settings, connection_id: &str) -> Option<Profile> {
    for conn in connections(settings) {
        if conn.id == connection_id {
            return Some(Profile {
                account: Account {
                    email: if !conn.email.is_empty() { conn.email } else { conn.name },
                    uuid: conn.id,
                },
                organization: Organization {
                    organization_type: if conn.provider.is_empty() { "9Router".into() } else { conn.provider },
                },
            });
        }
    }
    None
}
