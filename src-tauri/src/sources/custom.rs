use crate::error::{Error, Result};
use crate::http::{get_json_value_insecure, headers_from};
use crate::i18n::t;
use crate::settings::Settings;
use crate::sources::{as_f64, as_f64_opt, normalize_quotas, slug, utilization};
use crate::types::{Account, CustomCandidate, CustomField, CustomTestResult, Organization, Profile};
use serde_json::{json, Map, Value};
use std::collections::HashSet;
use url::Url;

const SKIP_KEYS: &[&str] = &["error", "auth_error", "rate_limited", "server_message", "retry_after"];
const USAGE_HINTS: &[&str] = &[
    "usage", "used", "util", "percent", "remaining", "quota", "credit", "limit", "spent", "count",
    "requests",
];

#[derive(Clone, Debug, PartialEq)]
enum Seg {
    Key(String),
    Index(usize),
}

pub fn usage(settings: &Settings, custom_id: &str) -> Map<String, Value> {
    let Some(custom) = settings.custom_sources.iter().find(|c| c.id == custom_id) else {
        return err(&t("no_token"));
    };
    if !valid_url(&custom.url) {
        return err(&t_fmt_code());
    }
    if custom.fields.is_empty() {
        match get_json_value_insecure(&custom.url, &auth_headers(&custom.header, &custom.token), 12) {
            Ok(Value::Object(data)) => {
                let quotas = data.get("quotas").and_then(Value::as_object).unwrap_or(&data);
                return normalize_quotas(quotas);
            }
            Ok(_) => return err(&t("connection_error")),
            Err(data) => return data,
        }
    }
    match get_json_value_insecure(&custom.url, &auth_headers(&custom.header, &custom.token), 12) {
        Ok(root) => {
            let parsed = extract(&root, &custom.fields);
            if parsed.is_empty() {
                err(&t("no_token"))
            } else {
                parsed
            }
        }
        Err(data) => data,
    }
}

pub fn profile(settings: &Settings, sid: &str, custom_id: &str) -> Option<Profile> {
    let custom = settings.custom_sources.iter().find(|c| c.id == custom_id)?;
    Some(Profile {
        account: Account { email: custom.name.clone(), uuid: sid.into() },
        organization: Organization { organization_type: "Custom".into() },
    })
}

pub fn probe(url: &str, header: &str, token: &str) -> Result<CustomTestResult> {
    if !valid_url(url) {
        return Err(Error::from("URL must be http(s)"));
    }
    match get_json_value_insecure(url, &auth_headers(header, token), 12) {
        Ok(root) => Ok(CustomTestResult {
            fields: discover(&root),
            keys: discover_all(&root),
            raw: pretty_raw(&root),
        }),
        Err(data) => {
            let msg = data
                .get("error")
                .and_then(Value::as_str)
                .unwrap_or("Request failed");
            Err(Error::from(msg.to_string()))
        }
    }
}

fn extract(root: &Value, fields: &[CustomField]) -> Map<String, Value> {
    let mut result = Map::new();
    let mut used = HashSet::new();
    for field in fields {
        let Some(value) = get_path(root, &field.path) else { continue };
        let key = unique_key(&field.key, &mut used);
        if let Some(entry) = value_to_entry(value, &field.label, &field.path) {
            result.insert(key, entry);
        }
    }
    result
}

fn discover(root: &Value) -> Vec<CustomCandidate> {
    let mut out = Vec::new();
    let mut used = HashSet::new();
    walk(root, &[], &mut out, &mut used, 0);
    out
}

fn walk(value: &Value, segs: &[Seg], out: &mut Vec<CustomCandidate>, used: &mut HashSet<String>, depth: usize) {
    if depth > 8 || out.len() >= 80 {
        return;
    }
    match value {
        Value::Object(obj) => {
            if is_quota_object(obj) {
                out.push(candidate_from_quota(obj, segs, used));
                return;
            }
            for (k, v) in obj {
                if SKIP_KEYS.contains(&k.as_str()) {
                    continue;
                }
                let mut next = segs.to_vec();
                next.push(Seg::Key(k.clone()));
                walk(v, &next, out, used, depth + 1);
            }
        }
        Value::Array(arr) => {
            for (i, v) in arr.iter().take(20).enumerate() {
                let mut next = segs.to_vec();
                next.push(Seg::Index(i));
                walk(v, &next, out, used, depth + 1);
            }
        }
        Value::Number(n) => {
            if segs.is_empty() {
                return;
            }
            let leaf = last_key(segs);
            if !is_usage_number_key(&leaf) {
                return;
            }
            let Some(v) = n.as_f64() else { return };
            out.push(candidate_from_number(v, segs, used));
        }
        _ => {}
    }
}

fn discover_all(root: &Value) -> Vec<CustomCandidate> {
    let mut out = Vec::new();
    let mut used = HashSet::new();
    walk_all(root, &[], &mut out, &mut used, 0);
    out
}

fn walk_all(value: &Value, segs: &[Seg], out: &mut Vec<CustomCandidate>, used: &mut HashSet<String>, depth: usize) {
    if depth > 8 || out.len() >= 120 {
        return;
    }
    match value {
        Value::Object(obj) => {
            if !segs.is_empty() && is_quota_object(obj) {
                out.push(candidate_from_quota(obj, segs, used));
            }
            for (k, v) in obj {
                if SKIP_KEYS.contains(&k.as_str()) {
                    continue;
                }
                let mut next = segs.to_vec();
                next.push(Seg::Key(k.clone()));
                walk_all(v, &next, out, used, depth + 1);
            }
        }
        Value::Array(arr) => {
            for (i, v) in arr.iter().take(20).enumerate() {
                let mut next = segs.to_vec();
                next.push(Seg::Index(i));
                walk_all(v, &next, out, used, depth + 1);
            }
        }
        Value::Number(n) => {
            if segs.is_empty() {
                return;
            }
            let Some(v) = n.as_f64() else { return };
            out.push(candidate_from_number(v, segs, used));
        }
        _ => {}
    }
}

fn is_quota_object(obj: &Map<String, Value>) -> bool {
    if as_f64_opt(obj.get("utilization").unwrap_or(&Value::Null)).is_some() {
        return true;
    }
    if as_f64_opt(obj.get("remainingPercentage").unwrap_or(&Value::Null)).is_some()
        || as_f64_opt(obj.get("remaining_percentage").unwrap_or(&Value::Null)).is_some()
    {
        return true;
    }
    let used = obj.contains_key("used") || obj.contains_key("used_credits");
    let total = obj.contains_key("total")
        || obj.contains_key("limit")
        || obj.contains_key("monthly_limit")
        || obj.contains_key("cap");
    if used && total {
        return true;
    }
    let remaining = obj.contains_key("remaining");
    remaining && total
}

fn is_usage_number_key(key: &str) -> bool {
    let k = key.to_lowercase();
    USAGE_HINTS.iter().any(|h| k.contains(h))
}

fn candidate_from_quota(obj: &Map<String, Value>, segs: &[Seg], used: &mut HashSet<String>) -> CustomCandidate {
    let path = format_path(segs);
    let raw_label = obj
        .get("label")
        .or_else(|| obj.get("quotaType"))
        .or_else(|| obj.get("name"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| title_from_path(segs));
    let key = unique_key(&slug(&raw_label), used);
    let util = utilization(obj);
    let mut preview = format!("{util:.0}%");
    let used_v = as_f64(obj.get("used"));
    let total_v = as_f64(obj.get("total").or_else(|| obj.get("limit")).or_else(|| obj.get("monthly_limit")));
    if total_v > 0.0 {
        preview = format!("{preview} · {used_v:.0}/{total_v:.0}");
    }
    CustomCandidate { path, key, label: raw_label, preview, kind: "quota".into() }
}

fn candidate_from_number(value: f64, segs: &[Seg], used: &mut HashSet<String>) -> CustomCandidate {
    let path = format_path(segs);
    let label = title_from_path(segs);
    let key = unique_key(&slug(&label), used);
    if is_percent_key(&last_key(segs)) {
        let util = as_percent(value);
        CustomCandidate {
            path,
            key,
            label,
            preview: format!("{util:.0}%"),
            kind: "percent".into(),
        }
    } else {
        CustomCandidate {
            path,
            key,
            label,
            preview: format!("{value}"),
            kind: "count".into(),
        }
    }
}

fn value_to_entry(value: &Value, label: &str, path: &str) -> Option<Value> {
    match value {
        Value::Object(obj) => {
            let mut entry = Map::new();
            entry.insert("utilization".into(), json!(utilization(obj)));
            entry.insert("resets_at".into(), json!(reset_at(obj)));
            let resolved = if label.trim().is_empty() {
                obj.get("label")
                    .or_else(|| obj.get("quotaType"))
                    .or_else(|| obj.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string()
            } else {
                label.to_string()
            };
            if !resolved.is_empty() {
                entry.insert("label".into(), json!(resolved));
            }
            if obj.get("invert").and_then(Value::as_bool) == Some(true) {
                entry.insert("invert".into(), json!(true));
            }
            Some(Value::Object(entry))
        }
        Value::Number(n) => number_entry(n.as_f64()?, label, path),
        Value::String(s) => s.parse::<f64>().ok().and_then(|v| number_entry(v, label, path)),
        _ => None,
    }
}

fn number_entry(value: f64, label: &str, path: &str) -> Option<Value> {
    let leaf = path.rsplit(['.', '[', ']']).find(|s| !s.is_empty()).unwrap_or(path);
    if is_percent_key(leaf) || is_percent_key(label) {
        Some(json!({
            "utilization": as_percent(value),
            "resets_at": "",
            "label": label,
        }))
    } else {
        Some(json!({
            "kind": "count",
            "used": value,
            "unit": "requests",
            "resets_at": "",
            "label": label,
        }))
    }
}

fn reset_at(obj: &Map<String, Value>) -> String {
    obj.get("resetAt")
        .or_else(|| obj.get("resets_at"))
        .or_else(|| obj.get("reset_at"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn is_percent_key(key: &str) -> bool {
    let k = key.to_lowercase();
    k.contains("percent") || k.contains("pct") || k.contains("util")
}


fn as_percent(value: f64) -> f64 {
    if (0.0..=1.0).contains(&value) {
        (value * 100.0).clamp(0.0, 100.0)
    } else {
        value.clamp(0.0, 100.0)
    }
}

fn unique_key(base: &str, used: &mut HashSet<String>) -> String {
    let key = if base.is_empty() { "quota".into() } else { base.to_string() };
    if used.insert(key.clone()) {
        return key;
    }
    let mut index = 2;
    loop {
        let next = format!("{key}_{index}");
        if used.insert(next.clone()) {
            return next;
        }
        index += 1;
    }
}

fn last_key(segs: &[Seg]) -> String {
    match segs.last() {
        Some(Seg::Key(k)) => k.clone(),
        Some(Seg::Index(i)) => i.to_string(),
        None => String::new(),
    }
}

fn title_from_path(segs: &[Seg]) -> String {
    let raw = last_key(segs);
    if raw.is_empty() {
        return "Quota".into();
    }
    raw.split(|c: char| c == '_' || c == '-' || c == '.')
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn parse_path(path: &str) -> Option<Vec<Seg>> {
    let b = path.as_bytes();
    let mut i = 0;
    let mut segs = Vec::new();
    while i < b.len() {
        if b[i] == b'.' {
            i += 1;
            continue;
        }
        if b[i] == b'[' {
            i += 1;
            if i < b.len() && (b[i] == b'"' || b[i] == b'\'') {
                let quote = b[i];
                i += 1;
                let start = i;
                while i < b.len() && b[i] != quote {
                    i += 1;
                }
                let key = std::str::from_utf8(&b[start..i]).ok()?.to_string();
                i += 1;
                if i >= b.len() || b[i] != b']' {
                    return None;
                }
                i += 1;
                segs.push(Seg::Key(key));
            } else {
                let start = i;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                let n = std::str::from_utf8(&b[start..i]).ok()?.parse().ok()?;
                if i >= b.len() || b[i] != b']' {
                    return None;
                }
                i += 1;
                segs.push(Seg::Index(n));
            }
        } else {
            let start = i;
            while i < b.len() && b[i] != b'.' && b[i] != b'[' {
                i += 1;
            }
            let key = std::str::from_utf8(&b[start..i]).ok()?.to_string();
            if key.is_empty() {
                return None;
            }
            segs.push(Seg::Key(key));
        }
    }
    if segs.is_empty() { None } else { Some(segs) }
}

fn format_path(segs: &[Seg]) -> String {
    let mut out = String::new();
    for (i, seg) in segs.iter().enumerate() {
        match seg {
            Seg::Key(k) if k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') => {
                if i > 0 {
                    out.push('.');
                }
                out.push_str(k);
            }
            Seg::Key(k) => {
                out.push_str(&format!("[\"{}\"]", k.replace('\\', "\\\\").replace('"', "\\\"")));
            }
            Seg::Index(n) => out.push_str(&format!("[{n}]")),
        }
    }
    out
}

fn get_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = root;
    for seg in parse_path(path)? {
        cur = match seg {
            Seg::Key(k) => cur.get(&k)?,
            Seg::Index(i) => cur.get(i)?,
        };
    }
    Some(cur)
}

fn auth_headers(header: &str, token: &str) -> reqwest::header::HeaderMap {
    let mut pairs = vec![("Accept", "application/json".to_string())];
    if !token.is_empty() {
        let header = if header.trim().is_empty() { "Authorization" } else { header };
        let value = if token.contains(' ') || header.to_lowercase() != "authorization" {
            token.to_string()
        } else {
            format!("Bearer {token}")
        };
        pairs.push((header, value));
    }
    let refs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    headers_from(&refs)
}

fn valid_url(url: &str) -> bool {
    Url::parse(url)
        .ok()
        .map(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some())
        == Some(true)
}

fn pretty_raw(value: &Value) -> String {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    const LIMIT: usize = 6000;
    if text.len() <= LIMIT {
        return text;
    }
    let mut cut = text.chars().take(LIMIT).collect::<String>();
    cut.push_str("\n…");
    cut
}

fn t_fmt_code() -> String {
    crate::i18n::t_fmt("http_error", &[("code", "?")])
}

fn err(msg: &str) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("error".into(), json!(msg));
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        json!({
            "quotas": {
                "five_hour": { "utilization": 48.0, "resets_at": "2026-09-05T18:00:00Z", "label": "Session" },
                "weekly": { "used": 12, "total": 100, "resetAt": "x" }
            },
            "remaining_percent": 31,
            "email": "user@example.com"
        })
    }

    #[test]
    fn discovers_quota_objects_and_usage_numbers() {
        let found = discover(&sample());
        let paths: Vec<_> = found.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"quotas.five_hour"));
        assert!(paths.contains(&"quotas.weekly"));
        assert!(paths.contains(&"remaining_percent"));
        assert!(!paths.iter().any(|p| p.contains("email")));
        let session = found.iter().find(|f| f.path == "quotas.five_hour").unwrap();
        assert_eq!(session.preview, "48%");
        let weekly = found.iter().find(|f| f.path == "quotas.weekly").unwrap();
        assert!(weekly.preview.contains("12%"));
    }

    #[test]
    fn extracts_only_selected_fields() {
        let fields = vec![
            CustomField { path: "quotas.five_hour".into(), key: "five_hour".into(), label: "Session".into() },
            CustomField { path: "remaining_percent".into(), key: "remaining".into(), label: "Left".into() },
        ];
        let parsed = extract(&sample(), &fields);
        assert_eq!(parsed["five_hour"]["utilization"], 48.0);
        assert_eq!(parsed["five_hour"]["label"], "Session");
        assert_eq!(parsed["remaining"]["utilization"], 31.0);
        assert_eq!(parsed["remaining"]["label"], "Left");
        assert!(!parsed.contains_key("weekly"));
    }

    #[test]
    fn lists_every_number_for_manual_pick() {
        let found = discover_all(&sample());
        let paths: Vec<_> = found.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"quotas.five_hour"));
        assert!(paths.contains(&"quotas.five_hour.utilization"));
        assert!(paths.contains(&"quotas.weekly.used"));
        assert!(paths.contains(&"remaining_percent"));
        assert!(!paths.iter().any(|p| p.contains("email")));
        let used = found.iter().find(|f| f.path == "quotas.weekly.used").unwrap();
        assert_eq!(used.kind, "count");
        assert_eq!(used.preview, "12");
    }

    #[test]
    fn parses_array_and_quoted_paths() {
        let root = json!({ "limits": [{ "used": 3, "total": 10 }], "Weekly usage": { "utilization": 9 } });
        assert_eq!(get_path(&root, "limits[0].used").and_then(Value::as_i64), Some(3));
        assert_eq!(
            get_path(&root, "[\"Weekly usage\"].utilization").and_then(Value::as_f64),
            Some(9.0)
        );
        let found = discover(&root);
        assert!(found.iter().any(|f| f.path == "limits[0]"));
    }
}
