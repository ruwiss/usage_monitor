use crate::cache::now;
use crate::i18n::t;
use crate::sources::{as_f64_opt, slug};
use crate::types::{Account, Organization, Profile};
use chrono::DateTime;
use parking_lot::Mutex;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const FETCH_TIMEOUT: Duration = Duration::from_secs(15);
const CACHE_SECS: f64 = 20.0;

struct Cache {
    at: f64,
    data: Option<Value>,
}

static CACHE: Mutex<Cache> = Mutex::new(Cache { at: 0.0, data: None });

pub struct OmpAccount {
    pub id: String,
    pub label: String,
    pub email: String,
}

pub fn installed() -> bool {
    omp_bin().is_some()
}

pub fn sources() -> Vec<OmpAccount> {
    let Some(root) = reports(false) else { return Vec::new() };
    parse_sources(&root)
}

pub fn has_source(payload: &str) -> bool {
    if payload.is_empty() || payload == "omp" {
        return installed();
    }
    sources().iter().any(|s| s.id == payload)
}

pub fn usage(payload: &str) -> Map<String, Value> {
    if !installed() {
        return err(&t("no_token"));
    }
    let Some(root) = reports(true) else {
        return err(&t("connection_error"));
    };
    let parsed = parse_usage(&root, payload);
    if parsed.contains_key("error") {
        return parsed;
    }
    if parsed.keys().all(|k| k.starts_with('_')) {
        return err(&t("no_token"));
    }
    parsed
}

pub fn profile(payload: &str) -> Option<Profile> {
    let root = reports(false)?;
    let accounts = parse_sources(&root);
    let account = if payload.is_empty() || payload == "omp" {
        accounts.into_iter().next()
    } else {
        accounts.into_iter().find(|a| a.id == payload)
    }?;
    Some(Profile {
        account: Account {
            email: account.email,
            uuid: format!("omp:{}", account.id),
        },
        organization: Organization {
            organization_type: account.label,
        },
    })
}

fn reports(force: bool) -> Option<Value> {
    let mut cache = CACHE.lock();
    let t = now();
    if !force {
        if let Some(data) = &cache.data {
            if t - cache.at < CACHE_SECS {
                return Some(data.clone());
            }
        }
    }
    match fetch_json() {
        Some(data) => {
            cache.at = t;
            cache.data = Some(data.clone());
            Some(data)
        }
        None => cache.data.clone(),
    }
}

fn omp_bin() -> Option<PathBuf> {
    if let Ok(custom) = std::env::var("USAGE_MONITOR_OMP") {
        let path = PathBuf::from(custom);
        if path.is_file() {
            return Some(path);
        }
    }
    if let Ok(path) = which::which("omp") {
        return Some(path);
    }
    let mut candidates = Vec::new();
    if let Some(local) = dirs::data_local_dir() {
        candidates.push(local.join("omp").join("omp.exe"));
        candidates.push(local.join("omp").join("omp"));
    }
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".local").join("bin").join("omp"));
        candidates.push(home.join(".local").join("bin").join("omp.exe"));
    }
    candidates.push(PathBuf::from("/opt/homebrew/bin/omp"));
    candidates.push(PathBuf::from("/usr/local/bin/omp"));
    candidates.into_iter().find(|p| p.is_file())
}

fn fetch_json() -> Option<Value> {
    let bin = omp_bin()?;
    let mut cmd = Command::new(bin);
    cmd.arg("usage")
        .arg("--json")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        stdout.read_to_end(&mut buf).ok();
        buf
    });
    let deadline = Instant::now() + FETCH_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let buf = reader.join().ok()?;
                if !status.success() {
                    return None;
                }
                return parse_json_bytes(&buf);
            }
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(40)),
            Err(_) => return None,
        }
    }
}

fn parse_json_bytes(buf: &[u8]) -> Option<Value> {
    let text = String::from_utf8_lossy(buf);
    let start = text.find('{')?;
    serde_json::from_str(text[start..].trim()).ok()
}

fn parse_sources(root: &Value) -> Vec<OmpAccount> {
    let Some(reports) = root.get("reports").and_then(Value::as_array) else {
        return Vec::new();
    };
    let counts = provider_counts(reports);
    let mut items = Vec::new();
    for report in reports {
        let Some(obj) = report.as_object() else { continue };
        if !has_usable_limit(obj) {
            continue;
        }
        let provider = obj.get("provider").and_then(Value::as_str).unwrap_or("").trim();
        if provider.is_empty() {
            continue;
        }
        let meta = obj.get("metadata").and_then(Value::as_object);
        let account_id = meta
            .and_then(|m| m.get("accountId"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        let email = meta
            .and_then(|m| m.get("email"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let id = source_payload(provider, account_id, &counts);
        items.push(OmpAccount {
            id,
            label: provider_title(provider),
            email,
        });
    }
    items
}

fn parse_usage(root: &Value, payload: &str) -> Map<String, Value> {
    let Some(reports) = root.get("reports").and_then(Value::as_array) else {
        return err(&t("connection_error"));
    };
    let counts = provider_counts(reports);
    let report = reports.iter().find_map(|item| {
        let obj = item.as_object()?;
        if matches_payload(obj, payload, &counts) {
            Some(obj)
        } else {
            None
        }
    });
    let Some(report) = report else {
        return err(&t("no_token"));
    };
    let mut result = Map::new();
    let mut used_names = std::collections::HashSet::new();
    if let Some(limits) = report.get("limits").and_then(Value::as_array) {
        for limit in limits {
            let Some(obj) = limit.as_object() else { continue };
            let Some((field, quota)) = quota_from_limit(obj) else { continue };
            let mut field = field;
            if used_names.contains(&field) {
                let mut index = 2;
                while used_names.contains(&format!("{field}_{index}")) {
                    index += 1;
                }
                field = format!("{field}_{index}");
            }
            used_names.insert(field.clone());
            result.insert(field, quota);
        }
    }
    let title = report
        .get("provider")
        .and_then(Value::as_str)
        .map(provider_title)
        .unwrap_or_else(|| "OMP".into());
    result.insert("_plan".into(), json!(title));
    result
}

fn matches_payload(report: &Map<String, Value>, payload: &str, counts: &HashMap<String, usize>) -> bool {
    if payload.is_empty() || payload == "omp" {
        return has_usable_limit(report);
    }
    let provider = report.get("provider").and_then(Value::as_str).unwrap_or("");
    let account_id = report
        .get("metadata")
        .and_then(Value::as_object)
        .and_then(|m| m.get("accountId"))
        .and_then(Value::as_str)
        .unwrap_or("");
    payload == source_payload(provider, account_id, counts) || payload == provider
}

fn provider_counts(reports: &[Value]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for report in reports {
        if let Some(provider) = report.get("provider").and_then(Value::as_str) {
            if !provider.is_empty() {
                *counts.entry(provider.to_string()).or_insert(0) += 1;
            }
        }
    }
    counts
}

fn source_payload(provider: &str, account_id: &str, counts: &HashMap<String, usize>) -> String {
    if counts.get(provider).copied().unwrap_or(0) > 1 && !account_id.is_empty() {
        format!("{provider}:{account_id}")
    } else {
        provider.to_string()
    }
}

fn has_usable_limit(report: &Map<String, Value>) -> bool {
    report
        .get("limits")
        .and_then(Value::as_array)
        .map(|limits| limits.iter().any(|item| item.as_object().and_then(quota_from_limit).is_some()))
        .unwrap_or(false)
}

fn quota_from_limit(limit: &Map<String, Value>) -> Option<(String, Value)> {
    let amount = limit.get("amount").and_then(Value::as_object)?;
    let window = limit.get("window").and_then(Value::as_object);
    let window_id = window.and_then(|w| w.get("id")).and_then(Value::as_str).unwrap_or("");
    let window_label = window.and_then(|w| w.get("label")).and_then(Value::as_str).unwrap_or("");
    let duration_ms = window.and_then(|w| w.get("durationMs")).and_then(as_f64_opt).unwrap_or(0.0);
    let period = period_slug(window_id, window_label, duration_ms);
    let label = limit.get("label").and_then(Value::as_str).unwrap_or("").trim();
    let extra = extra_slug(label);
    let field = if extra.is_empty() {
        period
    } else {
        format!("{period}_{extra}")
    };
    let resets_at = window
        .and_then(|w| w.get("resetsAt"))
        .and_then(as_f64_opt)
        .map(ms_to_iso)
        .unwrap_or_default();
    let pretty = if label.is_empty() { Value::Null } else { json!(label) };
    if let Some(utilization) = utilization_from(amount) {
        return Some((
            field,
            json!({
                "utilization": utilization,
                "resets_at": resets_at,
                "label": pretty,
                "invert": true,
            }),
        ));
    }
    let used = amount.get("used").and_then(as_f64_opt)?;
    if amount.get("limit").and_then(as_f64_opt).is_some() {
        return None;
    }
    let unit = amount
        .get("unit")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("requests");
    Some((
        field,
        json!({
            "kind": "count",
            "used": used,
            "unit": unit,
            "resets_at": resets_at,
            "label": pretty,
        }),
    ))
}

fn utilization_from(amount: &Map<String, Value>) -> Option<f64> {
    if let Some(frac) = amount.get("usedFraction").and_then(as_f64_opt) {
        return Some((frac * 100.0).clamp(0.0, 100.0));
    }
    if let Some(frac) = amount.get("remainingFraction").and_then(as_f64_opt) {
        return Some((100.0 - frac * 100.0).clamp(0.0, 100.0));
    }
    let used = amount.get("used").and_then(as_f64_opt)?;
    let limit = amount.get("limit").and_then(as_f64_opt)?;
    if limit <= 0.0 {
        return None;
    }
    Some((used / limit * 100.0).clamp(0.0, 100.0))
}

fn period_slug(window_id: &str, window_label: &str, duration_ms: f64) -> String {
    let id = window_id.to_ascii_lowercase();
    let label = window_label.to_ascii_lowercase();
    let five_h = 5.0 * 3600.0 * 1000.0;
    let day = 24.0 * 3600.0 * 1000.0;
    if matches_window(&id, &label, &["5h", "five_hour", "session"]) || near(duration_ms, five_h) {
        return "five_hour".into();
    }
    if matches_window(&id, &label, &["1w", "7d", "week"]) || near(duration_ms, 7.0 * day) {
        return "seven_day".into();
    }
    if matches_window(&id, &label, &["month"]) || near(duration_ms, 30.0 * day) || near(duration_ms, 31.0 * day) {
        return "thirty_day".into();
    }
    if matches_window(&id, &label, &["1d", "daily", "day"]) || near(duration_ms, day) {
        return "one_day".into();
    }
    let raw = if !id.is_empty() { id } else { label };
    let s = slug(&raw);
    if s.is_empty() { "quota".into() } else { s }
}

fn matches_window(id: &str, label: &str, hints: &[&str]) -> bool {
    hints.iter().any(|h| id.contains(h) || label.contains(h))
}

fn near(value: f64, target: f64) -> bool {
    value > 0.0 && (value - target).abs() <= target * 0.08
}

fn extra_slug(label: &str) -> String {
    let mut s = slug(label);
    for word in ["weekly", "monthly", "daily", "session", "credits", "credit", "hours", "hour", "days", "day", "percent"] {
        s = s.replace(word, "_");
    }
    slug(&s)
}

fn provider_title(id: &str) -> String {
    match id {
        "xai-oauth" => "Xai Oauth".into(),
        "cursor" => "Cursor".into(),
        other => other
            .split(|c: char| c == '-' || c == '_' || c.is_whitespace())
            .filter(|w| !w.is_empty())
            .map(|w| {
                let mut chars = w.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn ms_to_iso(ms: f64) -> String {
    let secs = (ms / 1000.0).floor() as i64;
    let nsecs = ((ms % 1000.0) * 1_000_000.0).clamp(0.0, 999_999_999.0) as u32;
    DateTime::from_timestamp(secs, nsecs)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
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
            "reports": [
                {
                    "provider": "xai-oauth",
                    "limits": [
                        {
                            "label": "SuperGrok Weekly Credits",
                            "window": { "id": "1w", "label": "Weekly", "durationMs": 604800000, "resetsAt": 1788699967062.0 },
                            "amount": { "used": 69, "limit": 100, "usedFraction": 0.69, "remainingFraction": 0.31, "unit": "percent" }
                        },
                        {
                            "label": "Grok Build (Weekly)",
                            "window": { "id": "1w", "label": "Weekly", "durationMs": 604800000, "resetsAt": 1788699967062.0 },
                            "amount": { "used": 68, "limit": 100, "usedFraction": 0.68, "unit": "percent" }
                        }
                    ],
                    "metadata": { "accountId": "acc-1", "email": "dev@example.com" }
                },
                {
                    "provider": "cursor",
                    "limits": [
                        {
                            "label": "gpt-4 requests",
                            "window": { "id": "monthly", "label": "Monthly", "resetsAt": 1789683060000.0 },
                            "amount": { "used": 0, "unit": "requests" }
                        },
                        {
                            "label": "Cursor Models",
                            "window": { "id": "monthly", "label": "Monthly", "resetsAt": 1789683060000.0 },
                            "amount": { "used": 5.09, "usedFraction": 0.0509, "unit": "percent" }
                        },
                        {
                            "label": "Other Models",
                            "window": { "id": "monthly", "label": "Monthly", "resetsAt": 1789683060000.0 },
                            "amount": { "used": 400, "limit": 400, "usedFraction": 1, "remainingFraction": 0, "unit": "usd" }
                        }
                    ],
                    "metadata": { "email": "dev@example.com" }
                }
            ]
        })
    }

    #[test]
    fn lists_providers() {
        let items = parse_sources(&sample());
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, "xai-oauth");
        assert_eq!(items[0].label, "Xai Oauth");
        assert_eq!(items[1].id, "cursor");
    }

    #[test]
    fn maps_omp_quotas() {
        let grok = parse_usage(&sample(), "xai-oauth");
        assert!((as_f64_opt(grok.get("seven_day_supergrok").unwrap().get("utilization").unwrap()).unwrap() - 69.0).abs() < 0.01);
        assert_eq!(grok["seven_day_supergrok"]["label"], "SuperGrok Weekly Credits");
        assert!(grok.contains_key("seven_day_grok_build"));
        assert_eq!(grok["_plan"], "Xai Oauth");

        let cursor = parse_usage(&sample(), "cursor");
        let req = cursor.get("thirty_day_gpt_4_requests").unwrap();
        assert_eq!(req["kind"], "count");
        assert_eq!(as_f64_opt(req.get("used").unwrap()).unwrap(), 0.0);
        assert_eq!(req["unit"], "requests");
        let models = cursor.get("thirty_day_cursor_models").unwrap();
        assert!((as_f64_opt(models.get("utilization").unwrap()).unwrap() - 5.09).abs() < 0.01);
        assert_eq!(models["invert"], true);
        assert_eq!(cursor["thirty_day_other_models"]["utilization"], 100.0);
    }

    #[test]
    fn maps_count_only_limits() {
        let (field, quota) = quota_from_limit(json!({
            "label": "gpt-4 requests",
            "amount": { "used": 0, "unit": "requests" }
        }).as_object().unwrap()).unwrap();
        assert_eq!(field, "quota_gpt_4_requests");
        assert_eq!(quota["kind"], "count");
        assert_eq!(as_f64_opt(quota.get("used").unwrap()).unwrap(), 0.0);
    }

    #[test]
    fn marks_percent_quotas_invert() {
        let grok = parse_usage(&sample(), "xai-oauth");
        assert_eq!(grok["seven_day_supergrok"]["invert"], true);
        assert!((as_f64_opt(grok.get("seven_day_supergrok").unwrap().get("utilization").unwrap()).unwrap() - 69.0).abs() < 0.01);
    }
}
