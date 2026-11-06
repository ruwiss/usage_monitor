use crate::http::{get_json, headers_from};
use crate::settings::Settings;
use crate::sources::{normalize_quotas, usage_providers};
use crate::types::{Account, Organization, Profile};
use parking_lot::Mutex;
use serde_json::{json, Map, Value};
use std::time::{Duration, Instant};

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

/// Short-lived memo of the /api/providers lookup so discovery (Settings, tray,
/// source switching) does not hit the network on every call. Also remembers
/// failures, so an unreachable router costs one timeout per window, not per call.
const CONNECTIONS_TTL: Duration = Duration::from_secs(5);

static CONNECTIONS_CACHE: Mutex<Option<(String, Instant, Vec<Conn>)>> = Mutex::new(None);

pub fn connections(settings: &Settings) -> Vec<Conn> {
    let base = base_url(settings);
    {
        let cache = CONNECTIONS_CACHE.lock();
        if let Some((url, at, conns)) = cache.as_ref() {
            if *url == base && at.elapsed() < CONNECTIONS_TTL {
                return conns.clone();
            }
        }
    }
    let conns = fetch_connections(&base);
    *CONNECTIONS_CACHE.lock() = Some((base, Instant::now(), conns.clone()));
    conns
}

fn fetch_connections(base: &str) -> Vec<Conn> {
    let url = format!("{base}/api/providers");
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

/// Connections of one provider, in display order.
pub fn provider_connections(settings: &Settings, provider: &str) -> Vec<Conn> {
    connections(settings)
        .into_iter()
        .filter(|c| c.provider == provider)
        .collect()
}

/// Which connection of a provider group is currently selected: the remembered
/// choice when it still exists, otherwise the first (active-first) connection.
pub fn resolve_group(settings: &Settings, provider: &str) -> Option<Conn> {
    let conns = provider_connections(settings, provider);
    if conns.is_empty() {
        return None;
    }
    if let Some(saved) = settings.group_selections.get(provider) {
        if let Some(conn) = conns.iter().find(|c| &c.id == saved) {
            return Some(conn.clone());
        }
    }
    Some(conns[0].clone())
}

impl Conn {
    pub fn display_name(&self) -> String {
        if !self.name.is_empty() {
            self.name.clone()
        } else if !self.email.is_empty() {
            self.email.clone()
        } else {
            self.id.chars().take(8).collect()
        }
    }
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
