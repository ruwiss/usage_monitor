pub mod claude;
pub mod codex;
pub mod custom;
pub mod grok;
pub mod ninerouter;
pub mod omp;

use crate::settings::Settings;
use crate::state::AppState;
use crate::types::{Profile, Source};
use regex::Regex;
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use std::sync::LazyLock;

static USAGE_PROVIDERS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    HashSet::from([
        "antigravity", "claude", "codebuddy-cn", "codebuddy-intl", "codex", "deepseek",
        "gemini-cli", "github", "glm", "glm-cn", "grok-cli", "groq", "iflow", "kimi",
        "kiro", "minimax", "minimax-cn", "ollama", "qoder", "vercel-ai-gateway", "zed",
    ])
});

const SESSION_HINTS: &[&str] = &["session", "five_hour", "5h", "5hr", "primary"];
const WEEKLY_HINTS: &[&str] = &["weekly", "seven_day", "7d", "7day", "secondary"];

pub fn usage_providers() -> &'static HashSet<&'static str> {
    &USAGE_PROVIDERS
}

pub fn list_sources(settings: &Settings) -> Vec<Source> {
    let mut items = Vec::new();
    if claude::token().is_some() {
        items.push(Source { id: "claude".into(), kind: "claude".into(), label: "Claude".into() });
    }
    if codex::token().is_some() {
        items.push(Source { id: "codex".into(), kind: "codex".into(), label: "Codex".into() });
    }
    if grok::has_account() {
        items.push(Source { id: "grok".into(), kind: "grok".into(), label: "Grok".into() });
    }
    if omp::installed() {
        let omp_sources = omp::sources();
        if omp_sources.is_empty() {
            items.push(Source { id: "omp".into(), kind: "omp".into(), label: "OMP".into() });
        } else {
            for acc in omp_sources {
                items.push(Source {
                    id: format!("omp:{}", acc.id),
                    kind: "omp".into(),
                    label: format!("OMP · {}", acc.label),
                });
            }
        }
    }
    for conn in ninerouter::connections(settings) {
        let name = if !conn.name.is_empty() { conn.name.clone() } else if !conn.email.is_empty() { conn.email.clone() } else { conn.id.chars().take(8).collect() };
        items.push(Source {
            id: format!("9r:{}", conn.id),
            kind: "9router".into(),
            label: format!("9Router · {}: {name}", if conn.provider.is_empty() { "provider" } else { &conn.provider }),
        });
    }
    for custom in &settings.custom_sources {
        items.push(Source {
            id: format!("custom:{}", custom.id),
            kind: "custom".into(),
            label: format!("Custom · {}", custom.name),
        });
    }
    items
}

pub fn current_source_id(state: &AppState) -> String {
    if let Some(id) = state.current_id.lock().clone() {
        return id;
    }
    let settings = state.settings.lock();
    let sid = settings.source_id.clone();
    if sid == "claude" && claude::token().is_none() {
        return default_source_id(&settings);
    }
    if sid == "codex" && codex::token().is_none() {
        return default_source_id(&settings);
    }
    if sid == "grok" && !grok::has_account() {
        return default_source_id(&settings);
    }
    if (sid == "omp" || sid.starts_with("omp:")) && !omp_source_available(&sid) {
        return default_source_id(&settings);
    }
    if sid.is_empty() {
        default_source_id(&settings)
    } else {
        sid
    }
}

fn default_source_id(settings: &Settings) -> String {
    list_sources(settings).into_iter().next().map(|s| s.id).unwrap_or_default()
}

pub fn select_source(state: &AppState, id: String) {
    *state.current_id.lock() = Some(id.clone());
    let _ = state.settings.lock().save_setting("source_id", json!(id));
}

pub fn read_access_token(state: &AppState) -> Option<String> {
    let id = current_source_id(state);
    if id.is_empty() { None } else { Some(id) }
}

pub fn fetch_usage(state: &AppState) -> Map<String, Value> {
    let sid = current_source_id(state);
    if sid.is_empty() {
        return error(&crate::i18n::t("no_token"));
    }
    let (kind, payload) = parse_id(&sid);
    match kind.as_str() {
        "claude" => claude::usage(),
        "codex" => codex::usage(),
        "grok" => grok::usage(),
        "omp" => omp::usage(&payload),
        "9router" => ninerouter::usage(&state.settings.lock(), &payload),
        "custom" => custom::usage(&state.settings.lock(), &payload),
        _ => error(&crate::i18n::t("no_token")),
    }
}

pub fn fetch_profile(state: &AppState) -> Option<Profile> {
    let sid = current_source_id(state);
    if sid.is_empty() {
        return None;
    }
    let (kind, payload) = parse_id(&sid);
    match kind.as_str() {
        "claude" => claude::profile(),
        "codex" => codex::profile(),
        "grok" => grok::profile(),
        "omp" => omp::profile(&payload),
        "9router" => ninerouter::profile(&state.settings.lock(), &payload),
        "custom" => custom::profile(&state.settings.lock(), &sid, &payload),
        _ => None,
    }
}

fn parse_id(source_id: &str) -> (String, String) {
    if let Some(rest) = source_id.strip_prefix("9r:") {
        ("9router".into(), rest.into())
    } else if let Some(rest) = source_id.strip_prefix("custom:") {
        ("custom".into(), rest.into())
    } else if let Some(rest) = source_id.strip_prefix("omp:") {
        ("omp".into(), rest.into())
    } else {
        (source_id.into(), source_id.into())
    }
}

fn omp_source_available(sid: &str) -> bool {
    if !omp::installed() {
        return false;
    }
    let payload = sid.strip_prefix("omp:").unwrap_or(sid);
    omp::has_source(payload)
}

fn error(msg: &str) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("error".into(), json!(msg));
    m
}

pub fn normalize_quotas(quotas: &Map<String, Value>) -> Map<String, Value> {
    let mut result = Map::new();
    let mut used = HashSet::new();
    for (name, quota) in quotas {
        let Some(q) = quota.as_object() else { continue };
        if q.get("unlimited").and_then(Value::as_bool) == Some(true) && q.get("total").is_none() {
            continue;
        }
        let mut field = canonical_field(name, q);
        if used.contains(&field) {
            let mut index = 2;
            while used.contains(&format!("{field}_{index}")) {
                index += 1;
            }
            field = format!("{field}_{index}");
        }
        used.insert(field.clone());
        result.insert(field, json!({
            "utilization": utilization(q),
            "resets_at": q.get("resetAt").or_else(|| q.get("resets_at")).or_else(|| q.get("reset_at")).and_then(Value::as_str).unwrap_or(""),
        }));
    }
    result
}

fn canonical_field(name: &str, quota: &Map<String, Value>) -> String {
    let raw = quota.get("quotaType").and_then(Value::as_str).unwrap_or(name);
    let slug = slug(if raw.is_empty() { "quota" } else { raw });
    if SESSION_HINTS.iter().any(|h| slug.contains(h)) {
        if slug.contains("review") { return "five_hour_review".into(); }
        if slug.contains("spark") { return "five_hour_spark".into(); }
        return "five_hour".into();
    }
    if WEEKLY_HINTS.iter().any(|h| slug.contains(h)) {
        if slug.contains("review") { return "seven_day_review".into(); }
        if slug.contains("spark") { return "seven_day_spark".into(); }
        let mut extra = slug.clone();
        for hint in WEEKLY_HINTS {
            extra = extra.replace(hint, "");
        }
        let extra = extra.trim_matches('_').to_string();
        let known = matches!(extra.as_str(), "sonnet" | "opus" | "haiku" | "fable" | "cowork");
        return if known { format!("seven_day_{extra}") } else { "seven_day".into() };
    }
    if slug.is_empty() { "quota".into() } else { slug }
}

pub fn slug(value: &str) -> String {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^a-z0-9]+").unwrap());
    RE.replace_all(&value.to_lowercase(), "_").trim_matches('_').to_string()
}

fn utilization(quota: &Map<String, Value>) -> f64 {
    if let Some(v) = quota.get("utilization").and_then(as_f64_opt) {
        return v.clamp(0.0, 100.0);
    }
    let used = as_f64(quota.get("used"));
    let total = as_f64(quota.get("total"));
    if total > 0.0 {
        return (used / total * 100.0).clamp(0.0, 100.0);
    }
    if let Some(rem) = quota.get("remainingPercentage").and_then(as_f64_opt) {
        return (100.0 - rem).clamp(0.0, 100.0);
    }
    if let Some(rem) = quota.get("remaining").and_then(as_f64_opt) {
        return (100.0 - rem).clamp(0.0, 100.0);
    }
    0.0
}

pub fn as_f64(value: Option<&Value>) -> f64 {
    as_f64_opt(value.unwrap_or(&Value::Null)).unwrap_or(0.0)
}

pub fn as_f64_opt(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse().ok(),
        Value::Object(o) if o.contains_key("val") => as_f64_opt(o.get("val").unwrap()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_weekly_hints() {
        let mut q = Map::new();
        q.insert("Weekly SuperGrok".into(), json!({"used": 54, "total": 100, "resetAt": "x"}));
        q.insert("5h session".into(), json!({"used": 10, "total": 100, "resetAt": "y"}));
        let n = normalize_quotas(&q);
        assert!(n.contains_key("seven_day"));
        assert!(n.contains_key("five_hour"));
    }
}
