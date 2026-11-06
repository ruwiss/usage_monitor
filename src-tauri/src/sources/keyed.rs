//! Key-based sources: provider accounts that are queried with an API key (or a token that is
//! auto-detected on this machine).
//!
//! Everything provider specific lives in `CATALOG`. A provider is a `ProviderDef` row plus one
//! `fetch_*` function (HTTP) and one `parse_*` function (pure JSON -> normalized usage map, unit
//! tested). Source ids are `key:<provider>:<id>`; `<id>` is `auto` for credentials detected on
//! this machine (GitHub CLI, OpenCode auth file) and a slug for manually added keys.

use crate::error::{Error, Result};
use crate::http::{get_json_value, headers_from};
use crate::i18n::t;
use crate::settings::Settings;
use crate::sources::{as_f64_opt, slug as quota_slug};
use crate::types::{Account, KeyProviderInfo, KeyedSource, Organization, Profile, Source};
use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};
use serde_json::{json, Map, Value};
use std::path::PathBuf;

const TIMEOUT: u64 = 12;
pub const AUTO_ID: &str = "auto";

type Usage = Map<String, Value>;

pub struct ProviderDef {
    pub kind: &'static str,
    pub name: &'static str,
    /// Placeholder shown in the key input.
    pub hint: &'static str,
    pub key_url: &'static str,
    pub dashboard_url: &'static str,
    /// Where the key may come from when the user leaves the input empty.
    pub auto_note: &'static str,
    /// Cheap local check (no network, no process spawn) whether credentials exist on this machine.
    pub detect: Option<fn() -> bool>,
    /// Resolves the credentials detected on this machine.
    pub auto_key: Option<fn() -> Option<String>>,
    pub fetch: fn(&str) -> Usage,
}

/// Add a provider: append a row here and write `fetch_x` + `parse_x`. Nothing else is needed.
pub static CATALOG: &[ProviderDef] = &[
    ProviderDef {
        kind: "openrouter",
        name: "OpenRouter",
        hint: "sk-or-...",
        key_url: "https://openrouter.ai/settings/keys",
        dashboard_url: "https://openrouter.ai/activity",
        auto_note: "",
        detect: None,
        auto_key: None,
        fetch: fetch_openrouter,
    },
    ProviderDef {
        kind: "copilot",
        name: "GitHub Copilot",
        hint: "ghp_... / gho_...",
        key_url: "https://github.com/settings/tokens",
        dashboard_url: "https://github.com/settings/copilot",
        auto_note: "GitHub CLI",
        detect: Some(gh_detected),
        auto_key: Some(gh_token),
        fetch: fetch_copilot,
    },
    ProviderDef {
        kind: "opencode-go",
        name: "OpenCode Go",
        hint: "API key",
        key_url: "https://opencode.ai/auth",
        dashboard_url: "https://opencode.ai/auth",
        auto_note: "OpenCode auth.json",
        detect: Some(|| opencode_key().is_some()),
        auto_key: Some(opencode_key),
        fetch: fetch_opencode_go,
    },
    ProviderDef {
        kind: "zai",
        name: "z.ai GLM",
        hint: "API key",
        key_url: "https://z.ai/manage-apikey/apikey-list",
        dashboard_url: "https://z.ai/manage-apikey/subscription",
        auto_note: "",
        detect: None,
        auto_key: None,
        fetch: fetch_zai,
    },
    ProviderDef {
        kind: "kimi",
        name: "Kimi Code",
        hint: "sk-kimi-...",
        key_url: "https://www.kimi.com/coding",
        dashboard_url: "https://www.kimi.com/coding",
        auto_note: "",
        detect: None,
        auto_key: None,
        fetch: fetch_kimi,
    },
    ProviderDef {
        kind: "minimax",
        name: "MiniMax",
        hint: "API key",
        key_url: "https://platform.minimax.io/user-center/basic-information/interface-key",
        dashboard_url: "https://platform.minimax.io/user-center/payment/coding-plan",
        auto_note: "",
        detect: None,
        auto_key: None,
        fetch: fetch_minimax,
    },
    ProviderDef {
        kind: "deepseek",
        name: "DeepSeek",
        hint: "sk-...",
        key_url: "https://platform.deepseek.com/api_keys",
        dashboard_url: "https://platform.deepseek.com/usage",
        auto_note: "",
        detect: None,
        auto_key: None,
        fetch: fetch_deepseek,
    },
    ProviderDef {
        kind: "moonshot",
        name: "Moonshot",
        hint: "sk-...",
        key_url: "https://platform.moonshot.ai/console/api-keys",
        dashboard_url: "https://platform.moonshot.ai/console/account",
        auto_note: "",
        detect: None,
        auto_key: None,
        fetch: fetch_moonshot,
    },
];

pub fn find(kind: &str) -> Option<&'static ProviderDef> {
    CATALOG.iter().find(|d| d.kind == kind)
}

pub fn provider_infos() -> Vec<KeyProviderInfo> {
    CATALOG
        .iter()
        .map(|d| KeyProviderInfo {
            kind: d.kind.into(),
            name: d.name.into(),
            hint: d.hint.into(),
            key_url: d.key_url.into(),
            key_optional: d.auto_key.is_some(),
        })
        .collect()
}

// ---- source listing -------------------------------------------------------------------------

/// Discovered keyed sources. Local checks only: no network, no child processes.
pub fn sources(settings: &Settings) -> Vec<Source> {
    let mut items = Vec::new();
    for def in CATALOG {
        if def.detect.map(|f| f()).unwrap_or(false) {
            items.push(Source {
                id: source_id(def.kind, AUTO_ID),
                kind: "keyed".into(),
                label: def.name.into(),
            });
        }
    }
    for entry in &settings.keyed_sources {
        let Some(def) = find(&entry.provider) else { continue };
        let name = entry.name.trim();
        let label = if name.is_empty() || name == def.name {
            def.name.to_string()
        } else {
            format!("{} \u{00b7} {name}", def.name)
        };
        items.push(Source { id: source_id(def.kind, &entry.id), kind: "keyed".into(), label });
    }
    items
}

pub fn source_id(provider: &str, id: &str) -> String {
    format!("key:{provider}:{id}")
}

/// `provider:id` payload of a `key:` source id.
fn split_payload(payload: &str) -> Option<(&'static ProviderDef, &str)> {
    let (provider, id) = payload.split_once(':')?;
    Some((find(provider)?, id))
}

/// Auto-detected entries cannot be removed, only hidden.
pub fn is_removable(source_id: &str) -> bool {
    source_id.strip_prefix("key:").and_then(|p| p.split_once(':')).map(|(_, id)| id != AUTO_ID).unwrap_or(false)
}

pub fn detail(source_id: &str) -> String {
    let Some(payload) = source_id.strip_prefix("key:") else { return String::new() };
    match split_payload(payload) {
        Some((def, AUTO_ID)) => def.auto_note.to_string(),
        _ => String::new(),
    }
}

pub fn exists(settings: &Settings, source_id: &str) -> bool {
    sources(settings).iter().any(|s| s.id == source_id)
}

// ---- credentials ------------------------------------------------------------------------------

fn resolve_key(settings: &Settings, def: &ProviderDef, id: &str) -> Option<String> {
    if id == AUTO_ID {
        return def.auto_key.and_then(|f| f());
    }
    let entry = settings.keyed_sources.iter().find(|e| e.provider == def.kind && e.id == id)?;
    let key = entry.key.trim();
    if key.is_empty() {
        def.auto_key.and_then(|f| f())
    } else {
        Some(key.to_string())
    }
}

pub fn usage(settings: &Settings, payload: &str) -> Usage {
    let Some((def, id)) = split_payload(payload) else { return err(&t("key_unknown_provider")) };
    let Some(key) = resolve_key(settings, def, id) else {
        return err(&t(if def.kind == "copilot" { "key_gh_missing" } else { "no_token" }));
    };
    run(def, &key)
}

fn run(def: &ProviderDef, key: &str) -> Usage {
    let mut data = (def.fetch)(key);
    if data.contains_key("error") {
        return data;
    }
    let has_data = data.iter().any(|(k, _)| !k.starts_with('_')) || data.contains_key("_credits") || data.contains_key("_info");
    if !has_data {
        return err(&t("key_no_data"));
    }
    data.insert("_links".into(), json!({ "dashboard_url": def.dashboard_url }));
    data
}

pub fn profile(settings: &Settings, sid: &str, payload: &str) -> Option<Profile> {
    let (def, id) = split_payload(payload)?;
    let name = settings
        .keyed_sources
        .iter()
        .find(|e| e.provider == def.kind && e.id == id)
        .map(|e| e.name.trim().to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| def.name.to_string());
    Some(Profile {
        account: Account { email: name, uuid: sid.into() },
        organization: Organization { organization_type: def.name.into() },
    })
}

/// Calls the provider once with `key`. `Ok` only when it returned usable data.
pub fn test(provider: &str, key: &str) -> Result<()> {
    let def = find(provider).ok_or_else(|| Error::from(t("key_unknown_provider")))?;
    let key = key.trim();
    let resolved = if key.is_empty() { def.auto_key.and_then(|f| f()) } else { Some(key.to_string()) };
    let Some(resolved) = resolved else {
        return Err(Error::from(t(if def.kind == "copilot" { "key_gh_missing" } else { "key_required" })));
    };
    let data = run(def, &resolved);
    match data.get("error").and_then(Value::as_str) {
        Some(msg) => {
            let extra = data.get("server_message").and_then(Value::as_str).unwrap_or("");
            Err(Error::from(if extra.is_empty() { msg.to_string() } else { format!("{msg} {extra}") }))
        }
        None => Ok(()),
    }
}

/// Picks an id for a new manual entry: slug of the name, unique, never `auto`.
pub fn new_entry_id(existing: &[KeyedSource], provider: &str, name: &str) -> String {
    let mut base: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .trim_matches('-')
        .to_string();
    if base.is_empty() || base == AUTO_ID {
        base = "key".into();
    }
    let taken = |id: &str| existing.iter().any(|e| e.provider == provider && e.id == id);
    let mut id = base.clone();
    let mut n = 2;
    while taken(&id) {
        id = format!("{base}-{n}");
        n += 1;
    }
    id
}

// ---- helpers ----------------------------------------------------------------------------------

fn err(msg: &str) -> Usage {
    let mut m = Map::new();
    m.insert("error".into(), json!(msg));
    m
}

fn key_rejected() -> Usage {
    err(&t("key_rejected"))
}

/// Raw GET; the error map still carries `http_status`/`auth_error` so callers can fall back.
fn raw_get(url: &str, headers: &[(&str, &str)]) -> std::result::Result<Value, Usage> {
    let mut all = vec![("User-Agent", "usage-monitor"), ("Accept", "application/json")];
    all.extend_from_slice(headers);
    get_json_value(url, &headers_from(&all), TIMEOUT)
}

fn is_rejected(e: &Usage) -> bool {
    e.get("auth_error").and_then(Value::as_bool) == Some(true)
        || matches!(e.get("http_status").and_then(Value::as_u64), Some(401 | 403))
}

fn finish_error(e: Usage) -> Usage {
    if is_rejected(&e) {
        key_rejected()
    } else {
        e
    }
}

fn get(url: &str, headers: &[(&str, &str)]) -> std::result::Result<Value, Usage> {
    raw_get(url, headers).map_err(finish_error)
}

fn bearer_get(url: &str, key: &str) -> std::result::Result<Value, Usage> {
    get(url, &[("Authorization", &format!("Bearer {key}"))])
}

fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key).and_then(as_f64_opt)
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(String::from)
}

/// ISO-8601 string from an epoch (seconds or milliseconds), RFC 3339 string or `YYYY-MM-DD`.
fn iso(v: Option<&Value>) -> String {
    let Some(v) = v else { return String::new() };
    let from_epoch = |x: f64| -> String {
        if x <= 0.0 {
            return String::new();
        }
        let secs = if x > 1e12 { (x / 1000.0) as i64 } else { x as i64 };
        Utc.timestamp_opt(secs, 0).single().map(|d| d.to_rfc3339()).unwrap_or_default()
    };
    match v {
        Value::Number(n) => n.as_f64().map(from_epoch).unwrap_or_default(),
        Value::String(s) => {
            let s = s.trim();
            if s.is_empty() {
                String::new()
            } else if let Ok(x) = s.parse::<f64>() {
                from_epoch(x)
            } else if DateTime::parse_from_rfc3339(s).is_ok() {
                s.to_string()
            } else if s.len() == 10 && s.as_bytes()[4] == b'-' {
                format!("{s}T00:00:00Z")
            } else if s.len() >= 19 {
                // "2025-12-01T00:00:00" without a zone: treat as UTC.
                format!("{}Z", s.trim_end_matches('Z'))
            } else {
                String::new()
            }
        }
        _ => String::new(),
    }
}

fn window(utilization: f64, resets_at: String, label: Option<&str>) -> Value {
    let mut w = json!({ "utilization": utilization.clamp(0.0, 100.0), "resets_at": resets_at });
    if let Some(l) = label {
        w["label"] = json!(l);
    }
    w
}

/// Inserts `entry` under `key`, or `key_2`, `key_3` ... when taken.
fn put(out: &mut Usage, key: &str, entry: Value) {
    let mut k = key.to_string();
    let mut n = 2;
    while out.contains_key(&k) {
        k = format!("{key}_{n}");
        n += 1;
    }
    out.insert(k, entry);
}

/// Normalized field name for a window length so labels and pace markers work
/// (`five_hour`, `seven_day`, `thirty_day`, ...). Odd lengths get a neutral key.
pub fn window_key(secs: i64) -> String {
    const WORDS: &[(&str, i64)] = &[
        ("one", 1), ("two", 2), ("three", 3), ("four", 4), ("five", 5), ("six", 6), ("seven", 7),
        ("eight", 8), ("nine", 9), ("ten", 10), ("eleven", 11), ("twelve", 12), ("thirty", 30),
    ];
    let word = |n: i64| WORDS.iter().find(|(_, v)| *v == n).map(|(w, _)| *w);
    if secs >= 3600 && secs % 86400 == 0 {
        if let Some(w) = word(secs / 86400) {
            return format!("{w}_day");
        }
    }
    if secs >= 3600 && secs % 3600 == 0 {
        if let Some(w) = word(secs / 3600) {
            return format!("{w}_hour");
        }
    }
    if (28 * 86400..=31 * 86400).contains(&secs) {
        return "thirty_day".into();
    }
    format!("window_{secs}")
}

fn money(v: f64, currency: &str) -> String {
    match currency {
        "" => format!("{v:.2}"),
        "USD" | "$" => format!("${v:.2}"),
        c => format!("{v:.2} {c}"),
    }
}

fn info(out: &mut Usage, label: String, value: String) {
    let list = out.entry("_info").or_insert_with(|| json!([]));
    if let Some(arr) = list.as_array_mut() {
        arr.push(json!({ "label": label, "value": value }));
    }
}

fn credits(balance: f64, currency: &str) -> Value {
    json!({ "balance": balance, "currency": currency })
}

// ---- OpenRouter -------------------------------------------------------------------------------

fn fetch_openrouter(key: &str) -> Usage {
    let key_json = match bearer_get("https://openrouter.ai/api/v1/key", key) {
        Ok(v) => v,
        Err(e) => return e,
    };
    // /credits needs a management-capable key on some accounts; the key endpoint alone is still useful.
    let credits_json = bearer_get("https://openrouter.ai/api/v1/credits", key).ok();
    parse_openrouter(&key_json, credits_json.as_ref(), Utc::now())
}

pub fn parse_openrouter(key_json: &Value, credits_json: Option<&Value>, now: DateTime<Utc>) -> Usage {
    let mut out = Usage::new();
    let k = key_json.get("data").unwrap_or(key_json);
    if k.get("is_free_tier").and_then(Value::as_bool) == Some(true) {
        out.insert("_plan".into(), json!("Free"));
    }
    let limit = num(k, "limit").unwrap_or(0.0);
    let mut balance_fallback = None;
    if limit > 0.0 {
        let remaining = num(k, "limit_remaining").unwrap_or(limit - num(k, "usage").unwrap_or(0.0));
        let reset_kind = text(k, "limit_reset");
        let (field, resets_at) = match reset_kind.as_deref() {
            Some("daily") => ("one_day", next_boundary(now, "daily")),
            Some("weekly") => ("seven_day", next_boundary(now, "weekly")),
            Some("monthly") => ("thirty_day", next_boundary(now, "monthly")),
            _ => ("key_limit", String::new()),
        };
        out.insert(field.into(), window((limit - remaining) / limit * 100.0, resets_at, Some(&t("keyed_key_limit"))));
        balance_fallback = Some((remaining, limit - remaining, limit));
    }
    let cr = credits_json.map(|c| c.get("data").unwrap_or(c));
    if let Some(c) = cr {
        if let Some(total) = num(c, "total_credits") {
            let used = num(c, "total_usage").unwrap_or(0.0);
            out.insert(
                "_credits".into(),
                json!({ "balance": total - used, "used": used, "limit": total, "currency": "USD" }),
            );
        }
    }
    if !out.contains_key("_credits") {
        if let Some((remaining, used, limit)) = balance_fallback {
            out.insert("_credits".into(), json!({ "balance": remaining, "used": used, "limit": limit, "currency": "USD" }));
        }
    }
    for (field, label) in [("usage_daily", "keyed_today"), ("usage_weekly", "keyed_week"), ("usage_monthly", "keyed_month")] {
        if let Some(v) = num(k, field) {
            info(&mut out, t(label), money(v, "USD"));
        }
    }
    out
}

/// Next UTC reset boundary for OpenRouter's `limit_reset` values.
fn next_boundary(now: DateTime<Utc>, kind: &str) -> String {
    let midnight = now.date_naive().and_hms_opt(0, 0, 0).unwrap();
    let next = match kind {
        "daily" => midnight + Duration::days(1),
        "weekly" => midnight + Duration::days(7 - now.weekday().num_days_from_monday() as i64),
        _ => {
            let (y, m) = if now.month() == 12 { (now.year() + 1, 1) } else { (now.year(), now.month() + 1) };
            chrono::NaiveDate::from_ymd_opt(y, m, 1).unwrap().and_hms_opt(0, 0, 0).unwrap()
        }
    };
    Utc.from_utc_datetime(&next).to_rfc3339()
}

// ---- GitHub Copilot ---------------------------------------------------------------------------

fn gh_config_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("GH_CONFIG_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir));
        }
    }
    if cfg!(windows) {
        return dirs::config_dir().map(|d| d.join("GitHub CLI"));
    }
    if let Ok(dir) = std::env::var("XDG_CONFIG_HOME") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir).join("gh"));
        }
    }
    dirs::home_dir().map(|h| h.join(".config").join("gh"))
}

fn env_token() -> Option<String> {
    ["GH_TOKEN", "GITHUB_TOKEN"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .map(|v| v.trim().to_string())
        .find(|v| !v.is_empty())
}

/// Cheap: an env var or a signed-in `gh` host file. Never starts a process.
fn gh_detected() -> bool {
    if env_token().is_some() {
        return true;
    }
    let Some(dir) = gh_config_dir() else { return false };
    std::fs::read_to_string(dir.join("hosts.yml")).map(|t| t.contains("github.com")).unwrap_or(false)
}

const GH_OK_TTL: std::time::Duration = std::time::Duration::from_secs(60);
const GH_FAIL_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// Last `gh auth token` outcome (None token = failure) and when it was obtained.
static GH_CACHE: parking_lot::Mutex<Option<(std::time::Instant, Option<String>)>> = parking_lot::Mutex::new(None);
/// True while a `gh` process is running, so at most one exists at a time.
static GH_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Cached outcome if still fresh: successes live longer than failures.
fn gh_cached(now: std::time::Instant) -> Option<Option<String>> {
    let cache = GH_CACHE.lock();
    let (at, tok) = cache.as_ref()?;
    let ttl = if tok.is_some() { GH_OK_TTL } else { GH_FAIL_TTL };
    (now.duration_since(*at) < ttl).then(|| tok.clone())
}

fn gh_store(tok: Option<String>) {
    *GH_CACHE.lock() = Some((std::time::Instant::now(), tok));
}

fn gh_token() -> Option<String> {
    if let Some(tok) = env_token() {
        return Some(tok);
    }
    if let Some(cached) = gh_cached(std::time::Instant::now()) {
        return cached;
    }
    // Another poll/caller already has a `gh` process in flight: do not start a second one.
    if GH_BUSY.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return GH_CACHE.lock().as_ref().and_then(|(_, tok)| tok.clone());
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut cmd = std::process::Command::new("gh");
        cmd.args(["auth", "token"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        crate::platform::no_window(&mut cmd);
        let tok = cmd
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|t| !t.is_empty());
        gh_store(tok.clone());
        GH_BUSY.store(false, std::sync::atomic::Ordering::SeqCst);
        let _ = tx.send(tok);
    });
    match rx.recv_timeout(std::time::Duration::from_secs(8)) {
        Ok(tok) => tok,
        Err(_) => {
            // Hung `gh`: back off for the failure TTL; the thread keeps GH_BUSY until it exits.
            gh_store(None);
            None
        }
    }
}

fn fetch_copilot(key: &str) -> Usage {
    let auth = format!("token {key}");
    match get(
        "https://api.github.com/copilot_internal/user",
        &[
            ("Authorization", &auth),
            ("Editor-Version", "vscode/1.100.0"),
            ("Editor-Plugin-Version", "copilot-chat/0.30.0"),
            ("X-GitHub-Api-Version", "2025-04-01"),
        ],
    ) {
        Ok(v) => parse_copilot(&v),
        Err(e) => e,
    }
}

pub fn parse_copilot(j: &Value) -> Usage {
    let mut out = Usage::new();
    if let Some(plan) = text(j, "copilot_plan").or_else(|| text(j, "access_type_sku")) {
        out.insert("_plan".into(), json!(plan));
    }
    let reset = {
        let utc = iso(j.get("quota_reset_date_utc"));
        if utc.is_empty() { iso(j.get("quota_reset_date")) } else { utc }
    };
    let mut unlimited_any = false;
    if let Some(q) = j.get("quota_snapshots").and_then(Value::as_object) {
        for (name, field, label) in [
            ("premium_interactions", "thirty_day", "keyed_premium"),
            ("chat", "copilot_chat", "keyed_chat"),
            ("completions", "copilot_completions", "keyed_completions"),
        ] {
            let Some(w) = q.get(name) else { continue };
            if w.get("unlimited").and_then(Value::as_bool) == Some(true) {
                unlimited_any = true;
                continue;
            }
            let remaining_pct = num(w, "percent_remaining").or_else(|| {
                let ent = num(w, "entitlement")?;
                (ent > 0.0).then(|| num(w, "remaining").unwrap_or(0.0) / ent * 100.0)
            });
            let Some(remaining_pct) = remaining_pct else { continue };
            out.insert(field.into(), window(100.0 - remaining_pct, reset.clone(), Some(&t(label))));
        }
    }
    if out.keys().all(|k| k.starts_with('_')) && unlimited_any {
        info(&mut out, t("keyed_premium"), t("credits_unlimited"));
    }
    if let Some(login) = text(j, "login") {
        info(&mut out, t("keyed_account"), login);
    }
    out
}

// ---- OpenCode Go ------------------------------------------------------------------------------

fn opencode_auth_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(home) = dirs::home_dir() {
        paths.push(home.join(".local").join("share").join("opencode").join("auth.json"));
    }
    if let Some(data) = dirs::data_dir() {
        paths.push(data.join("opencode").join("auth.json"));
    }
    paths
}

/// Key of OpenCode's `opencode-go` provider from its auth.json. File read only.
fn opencode_key() -> Option<String> {
    for path in opencode_auth_paths() {
        let Ok(text) = std::fs::read_to_string(path) else { continue };
        if let Some(key) = parse_opencode_auth(&text) {
            return Some(key);
        }
    }
    None
}

pub fn parse_opencode_auth(text: &str) -> Option<String> {
    let v: Value = serde_json::from_str(text).ok()?;
    let entry = v.get("opencode-go")?;
    self::text(entry, "key").or_else(|| self::text(entry, "access"))
}

fn fetch_opencode_go(key: &str) -> Usage {
    match bearer_get("https://opencode.ai/zen/go/v1/usage", key) {
        Ok(v) => parse_opencode_go(&v),
        Err(e) => e,
    }
}

pub fn parse_opencode_go(j: &Value) -> Usage {
    let mut out = Usage::new();
    out.insert("_plan".into(), json!("Go"));
    let u = j.get("usage").unwrap_or(j);
    for (name, field) in [("rolling", "five_hour"), ("weekly", "seven_day"), ("monthly", "thirty_day")] {
        let Some(w) = u.get(name) else { continue };
        out.insert(field.into(), window(num(w, "percent").unwrap_or(0.0), iso(w.get("resetsAt")), None));
        if let Some(status) = text(w, "status").filter(|s| s != "ok") {
            info(&mut out, t("keyed_status"), format!("{name}: {status}"));
        }
    }
    out
}

// ---- z.ai GLM ---------------------------------------------------------------------------------

fn fetch_zai(key: &str) -> Usage {
    let h = [("Authorization", key)];
    let raw = match raw_get("https://api.z.ai/api/monitor/usage/quota/limit", &h) {
        Ok(v) => Ok(v),
        // Keys of the mainland platform are not valid on the international host.
        Err(e) if is_rejected(&e) || e.get("http_status").and_then(Value::as_u64) == Some(404) => {
            raw_get("https://open.bigmodel.cn/api/monitor/usage/quota/limit", &h)
        }
        Err(e) => Err(e),
    };
    match raw {
        Ok(v) => parse_zai(&v),
        Err(e) => finish_error(e),
    }
}

pub fn parse_zai(j: &Value) -> Usage {
    let mut out = Usage::new();
    let data = j.get("data").unwrap_or(j);
    if let Some(plan) = text(data, "level").or_else(|| text(data, "planName")) {
        out.insert("_plan".into(), json!(plan));
    }
    for l in data.get("limits").and_then(Value::as_array).into_iter().flatten() {
        let kind = text(l, "type").unwrap_or_default();
        let used = num(l, "percentage").unwrap_or_else(|| match num(l, "usage") {
            Some(total) if total > 0.0 => num(l, "currentValue").unwrap_or(0.0) / total * 100.0,
            _ => 0.0,
        });
        let resets = iso(l.get("nextResetTime"));
        if kind.contains("TOKEN") {
            put(&mut out, "five_hour", window(used, resets, None));
        } else if kind.contains("TIME") {
            put(&mut out, "thirty_day", window(used, resets, Some(&t("keyed_tool_calls"))));
        } else if !kind.is_empty() {
            put(&mut out, &quota_slug(&kind), window(used, resets, Some(&kind)));
        }
    }
    out
}

// ---- Kimi Code --------------------------------------------------------------------------------

fn fetch_kimi(key: &str) -> Usage {
    match bearer_get("https://api.kimi.com/coding/v1/usages", key) {
        Ok(v) => parse_kimi(&v),
        Err(e) => e,
    }
}

fn kimi_window(out: &mut Usage, field: &str, d: &Value) {
    let limit = num(d, "limit").unwrap_or(0.0);
    if limit <= 0.0 {
        return;
    }
    let remaining = num(d, "remaining").unwrap_or(limit - num(d, "used").unwrap_or(0.0));
    let resets = {
        let a = iso(d.get("resetTime"));
        if a.is_empty() { iso(d.get("reset_time")) } else { a }
    };
    put(out, field, window((limit - remaining) / limit * 100.0, resets, None));
}

pub fn parse_kimi(j: &Value) -> Usage {
    let mut out = Usage::new();
    for l in j.get("limits").and_then(Value::as_array).into_iter().flatten() {
        let w = l.get("window");
        let dur = w.and_then(|w| num(w, "duration")).unwrap_or(0.0) as i64;
        let unit = w.and_then(|w| text(w, "timeUnit")).unwrap_or_default();
        let secs = if unit.contains("MINUTE") {
            dur * 60
        } else if unit.contains("HOUR") {
            dur * 3600
        } else if unit.contains("DAY") {
            dur * 86400
        } else {
            0
        };
        let field = if secs > 0 { window_key(secs) } else { "kimi_window".into() };
        if let Some(d) = l.get("detail") {
            kimi_window(&mut out, &field, d);
        }
    }
    if let Some(total) = j.get("usage").filter(|u| u.is_object()) {
        kimi_window(&mut out, "seven_day", total);
    }
    out
}

// ---- MiniMax ----------------------------------------------------------------------------------

fn fetch_minimax(key: &str) -> Usage {
    let auth = format!("Bearer {key}");
    let hdr = [("Authorization", auth.as_str())];
    let raw = match raw_get("https://api.minimax.io/v1/api/openplatform/coding_plan/remains", &hdr) {
        Ok(v) => Ok(v),
        Err(e) if is_rejected(&e) || e.get("http_status").and_then(Value::as_u64) == Some(404) => {
            raw_get("https://api.minimaxi.com/v1/api/openplatform/coding_plan/remains", &hdr)
        }
        Err(e) => Err(e),
    };
    match raw {
        Ok(v) => parse_minimax(&v),
        Err(e) => finish_error(e),
    }
}

pub fn parse_minimax(j: &Value) -> Usage {
    let mut out = Usage::new();
    // A non-zero base_resp.status_code means the key was accepted but the call failed.
    if let Some(code) = j.get("base_resp").and_then(|b| b.get("status_code")).and_then(Value::as_i64) {
        if code != 0 {
            let msg = j.get("base_resp").and_then(|b| text(b, "status_msg")).unwrap_or_default();
            return if code == 1004 || code == 2049 { key_rejected() } else { err(&msg) };
        }
    }
    for m in j.get("model_remains").and_then(Value::as_array).into_iter().flatten().take(6) {
        let name = text(m, "model_name").unwrap_or_else(|| "Model".into());
        let model = quota_slug(&name);
        // MiniMax reports the REMAINING count in *_usage_count.
        let mut add = |total_key: &str, remaining_key: &str, start_key: &str, end_key: &str, default_secs: i64| {
            let total = num(m, total_key).unwrap_or(0.0);
            if total <= 0.0 {
                return;
            }
            let remaining = num(m, remaining_key).unwrap_or(total);
            let start = iso(m.get(start_key));
            let end = iso(m.get(end_key));
            let secs = match (DateTime::parse_from_rfc3339(&start), DateTime::parse_from_rfc3339(&end)) {
                (Ok(a), Ok(b)) => (b - a).num_seconds().max(0),
                _ => default_secs,
            };
            let base = if secs > 0 { window_key(secs) } else { "window".into() };
            let used = (total - remaining) / total * 100.0;
            out.insert(format!("{base}_{model}"), window(used, end, Some(&name)));
        };
        add("current_interval_total_count", "current_interval_usage_count", "start_time", "end_time", 0);
        add("current_weekly_total_count", "current_weekly_usage_count", "weekly_start_time", "weekly_end_time", 7 * 86400);
    }
    out
}

// ---- DeepSeek ---------------------------------------------------------------------------------

fn fetch_deepseek(key: &str) -> Usage {
    match bearer_get("https://api.deepseek.com/user/balance", key) {
        Ok(v) => parse_deepseek(&v),
        Err(e) => e,
    }
}

pub fn parse_deepseek(j: &Value) -> Usage {
    let mut out = Usage::new();
    for (i, b) in j.get("balance_infos").and_then(Value::as_array).into_iter().flatten().enumerate() {
        let cur = text(b, "currency").unwrap_or_default();
        let total = num(b, "total_balance").unwrap_or(0.0);
        if i == 0 {
            out.insert("_credits".into(), credits(total, &cur));
        } else {
            info(&mut out, format!("{} ({cur})", t("credits_balance")), money(total, &cur));
        }
        if let Some(g) = num(b, "granted_balance").filter(|g| *g > 0.0) {
            info(&mut out, t("keyed_granted"), money(g, &cur));
        }
        if let Some(tp) = num(b, "topped_up_balance") {
            info(&mut out, t("keyed_topped_up"), money(tp, &cur));
        }
    }
    if j.get("is_available").and_then(Value::as_bool) == Some(false) {
        info(&mut out, t("keyed_status"), t("keyed_balance_low"));
    }
    out
}

// ---- Moonshot ---------------------------------------------------------------------------------

fn fetch_moonshot(key: &str) -> Usage {
    let auth = format!("Bearer {key}");
    let h = [("Authorization", auth.as_str())];
    // moonshot.ai bills in USD, the mainland moonshot.cn host in CNY.
    let (raw, currency) = match raw_get("https://api.moonshot.ai/v1/users/me/balance", &h) {
        Ok(v) => (Ok(v), "USD"),
        Err(e) if is_rejected(&e) => (raw_get("https://api.moonshot.cn/v1/users/me/balance", &h), "CNY"),
        Err(e) => (Err(e), "USD"),
    };
    match raw {
        Ok(v) => parse_moonshot(&v, currency),
        Err(e) => finish_error(e),
    }
}

pub fn parse_moonshot(j: &Value, currency: &str) -> Usage {
    let mut out = Usage::new();
    let d = j.get("data").unwrap_or(j);
    if let Some(a) = num(d, "available_balance") {
        out.insert("_credits".into(), credits(a, currency));
    }
    if let Some(v) = num(d, "voucher_balance").filter(|v| *v > 0.0) {
        info(&mut out, t("keyed_voucher"), money(v, currency));
    }
    if let Some(c) = num(d, "cash_balance") {
        info(&mut out, t("keyed_cash"), money(c, currency));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gh_cache_ttl_distinguishes_success_and_failure() {
        let start = std::time::Instant::now();
        gh_store(Some("tok".into()));
        assert_eq!(gh_cached(start), Some(Some("tok".into())));
        assert_eq!(gh_cached(start + GH_OK_TTL + std::time::Duration::from_secs(1)), None);
        gh_store(None);
        assert_eq!(gh_cached(start), Some(None));
        assert_eq!(gh_cached(start + GH_FAIL_TTL + std::time::Duration::from_secs(1)), None);
        *GH_CACHE.lock() = None;
    }

    fn fixed_now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 4, 15, 30, 0).unwrap() // a Wednesday
    }

    fn util(u: &Usage, key: &str) -> f64 {
        u[key]["utilization"].as_f64().unwrap_or(-1.0)
    }

    #[test]
    fn window_keys() {
        assert_eq!(window_key(18000), "five_hour");
        assert_eq!(window_key(604800), "seven_day");
        assert_eq!(window_key(30 * 86400), "thirty_day");
        assert_eq!(window_key(86400), "one_day");
        assert_eq!(window_key(1234), "window_1234");
    }

    #[test]
    fn openrouter_key_limit_and_credits() {
        let key = json!({"data": {
            "label": "sk-or-v1-abc...", "limit": 50.0, "limit_remaining": 37.5, "limit_reset": "monthly",
            "usage": 12.5, "usage_daily": 0.4, "usage_weekly": 3.1, "usage_monthly": 12.5, "is_free_tier": false
        }});
        let credits = json!({"data": {"total_credits": 100.5, "total_usage": 25.25}});
        let u = parse_openrouter(&key, Some(&credits), fixed_now());
        assert_eq!(util(&u, "thirty_day"), 25.0);
        assert_eq!(u["thirty_day"]["resets_at"], "2026-12-01T00:00:00+00:00");
        assert_eq!(u["_credits"]["balance"], 75.25);
        assert_eq!(u["_credits"]["limit"], 100.5);
        assert_eq!(u["_credits"]["currency"], "USD");
        assert_eq!(u["_info"].as_array().unwrap().len(), 3);
        assert!(!u.contains_key("_plan"));
    }

    #[test]
    fn openrouter_unlimited_key_credits_only_and_free_tier() {
        let key = json!({"data": {"limit": null, "limit_remaining": null, "usage": 1.0, "is_free_tier": true}});
        let credits = json!({"data": {"total_credits": 10, "total_usage": 4}});
        let u = parse_openrouter(&key, Some(&credits), fixed_now());
        assert_eq!(u["_plan"], "Free");
        assert!(!u.keys().any(|k| !k.starts_with('_')), "no bar without a real limit");
        assert_eq!(u["_credits"]["balance"], 6.0);
        // Credits endpoint unavailable: still no bar and no credits.
        assert!(!parse_openrouter(&key, None, fixed_now()).contains_key("_credits"));
    }

    #[test]
    fn openrouter_reset_boundaries() {
        assert_eq!(next_boundary(fixed_now(), "daily"), "2026-11-05T00:00:00+00:00");
        assert_eq!(next_boundary(fixed_now(), "weekly"), "2026-11-09T00:00:00+00:00");
        let dec = Utc.with_ymd_and_hms(2026, 12, 31, 23, 0, 0).unwrap();
        assert_eq!(next_boundary(dec, "monthly"), "2027-01-01T00:00:00+00:00");
    }

    #[test]
    fn copilot_premium_chat_and_plan() {
        let j = json!({
            "login": "octocat", "copilot_plan": "individual_pro", "access_type_sku": "copilot_pro_subscriber",
            "quota_reset_date": "2026-12-01", "quota_reset_date_utc": "2026-12-01T00:00:00.000Z",
            "quota_snapshots": {
                "premium_interactions": {"entitlement": 300, "remaining": 225, "percent_remaining": 75.0, "unlimited": false, "overage_permitted": true, "quota_id": "premium_interactions"},
                "chat": {"entitlement": 0, "remaining": 0, "percent_remaining": 100.0, "unlimited": true, "quota_id": "chat"},
                "completions": {"entitlement": 0, "remaining": 0, "percent_remaining": 100.0, "unlimited": true, "quota_id": "completions"}
            }
        });
        let u = parse_copilot(&j);
        assert_eq!(u["_plan"], "individual_pro");
        assert_eq!(util(&u, "thirty_day"), 25.0);
        assert_eq!(u["thirty_day"]["resets_at"], "2026-12-01T00:00:00.000Z");
        assert!(!u.contains_key("copilot_chat"), "unlimited quotas get no bar");
        assert_eq!(u["_info"][0]["value"], "octocat");
    }

    #[test]
    fn copilot_percent_from_entitlement_and_date_only_reset() {
        let j = json!({"quota_reset_date": "2026-12-01", "quota_snapshots": {
            "premium_interactions": {"entitlement": 50, "remaining": 10, "unlimited": false}}});
        let u = parse_copilot(&j);
        assert_eq!(util(&u, "thirty_day"), 80.0);
        assert_eq!(u["thirty_day"]["resets_at"], "2026-12-01T00:00:00Z");
    }

    #[test]
    fn copilot_all_unlimited_shows_info() {
        let j = json!({"quota_snapshots": {"chat": {"unlimited": true}, "premium_interactions": {"unlimited": true}}});
        let u = parse_copilot(&j);
        assert!(u.contains_key("_info"));
    }

    #[test]
    fn opencode_go_windows() {
        let j = json!({"usage": {
            "rolling": {"status": "ok", "resetInSec": 3600, "resetsAt": "2026-11-04T17:00:00.000Z", "percent": 42},
            "weekly": {"status": "ok", "resetsAt": "2026-11-09T00:00:00.000Z", "percent": 10.5},
            "monthly": {"status": "rate-limited", "resetsAt": "2026-12-01T00:00:00.000Z", "percent": 100}
        }});
        let u = parse_opencode_go(&j);
        assert_eq!(util(&u, "five_hour"), 42.0);
        assert_eq!(util(&u, "seven_day"), 10.5);
        assert_eq!(util(&u, "thirty_day"), 100.0);
        assert_eq!(u["five_hour"]["resets_at"], "2026-11-04T17:00:00.000Z");
        assert_eq!(u["_plan"], "Go");
        assert_eq!(u["_info"].as_array().unwrap().len(), 1, "only non-ok statuses are reported");
    }

    #[test]
    fn opencode_auth_file() {
        let text = r#"{"anthropic": {"type": "oauth", "access": "x"}, "opencode-go": {"type": "api", "key": "sk-go-123"}}"#;
        assert_eq!(parse_opencode_auth(text).as_deref(), Some("sk-go-123"));
        assert!(parse_opencode_auth(r#"{"openai": {"key": "k"}}"#).is_none());
        assert!(parse_opencode_auth("not json").is_none());
    }

    #[test]
    fn zai_limits() {
        let j = json!({"code": 200, "msg": "ok", "success": true, "data": {"level": "pro", "limits": [
            {"type": "TOKENS_LIMIT", "unit": 3, "number": 5, "percentage": 37, "nextResetTime": 1793811600000i64},
            {"type": "TIME_LIMIT", "unit": 5, "number": 1, "usage": 1000, "currentValue": 250, "remaining": 750, "percentage": 25, "nextResetTime": 1796083200000i64}
        ]}});
        let u = parse_zai(&j);
        assert_eq!(u["_plan"], "pro");
        assert_eq!(util(&u, "five_hour"), 37.0);
        assert_eq!(util(&u, "thirty_day"), 25.0);
        assert!(u["five_hour"]["resets_at"].as_str().unwrap().starts_with("2026-11-0"));
    }

    #[test]
    fn zai_percentage_from_usage_when_missing() {
        let j = json!({"data": {"limits": [{"type": "TIME_LIMIT", "usage": 200, "currentValue": 50}]}});
        assert_eq!(util(&parse_zai(&j), "thirty_day"), 25.0);
    }

    #[test]
    fn kimi_windows_and_total() {
        let j = json!({
            "user": {"userId": "u1"},
            "usage": {"limit": "100", "remaining": "60", "resetTime": "2026-11-09T00:00:00Z"},
            "limits": [{"window": {"duration": 300, "timeUnit": "TIME_UNIT_MINUTE"},
                        "detail": {"limit": "100", "remaining": "90", "resetTime": "2026-11-04T18:00:00Z"}}]
        });
        let u = parse_kimi(&j);
        assert_eq!(util(&u, "five_hour"), 10.0);
        assert_eq!(util(&u, "seven_day"), 40.0);
        assert_eq!(u["seven_day"]["resets_at"], "2026-11-09T00:00:00Z");
    }

    #[test]
    fn kimi_duplicate_window_gets_suffix() {
        let d = json!({"limit": 10, "remaining": 5});
        let j = json!({"usage": d, "limits": [{"window": {"duration": 7, "timeUnit": "TIME_UNIT_DAY"}, "detail": d}]});
        let u = parse_kimi(&j);
        assert!(u.contains_key("seven_day") && u.contains_key("seven_day_2"));
    }

    #[test]
    fn minimax_models() {
        let j = json!({"base_resp": {"status_code": 0, "status_msg": "success"}, "model_remains": [
            {"start_time": 1793797200000i64, "end_time": 1793815200000i64, "remains_time": 3600000,
             "current_interval_total_count": 1500, "current_interval_usage_count": 1200, "model_name": "MiniMax-M2"},
            {"start_time": 1793797200000i64, "end_time": 1793815200000i64,
             "current_interval_total_count": 0, "current_interval_usage_count": 0, "model_name": "speech-hd"}
        ]});
        let u = parse_minimax(&j);
        assert_eq!(u.len(), 1);
        assert_eq!(util(&u, "five_hour_minimax_m2"), 20.0);
        assert_eq!(u["five_hour_minimax_m2"]["label"], "MiniMax-M2");
    }

    #[test]
    fn minimax_error_status() {
        let j = json!({"base_resp": {"status_code": 1004, "status_msg": "login fail"}});
        assert!(parse_minimax(&j).contains_key("error"));
    }

    #[test]
    fn deepseek_balance() {
        let j = json!({"is_available": true, "balance_infos": [
            {"currency": "CNY", "total_balance": "110.00", "granted_balance": "10.00", "topped_up_balance": "100.00"},
            {"currency": "USD", "total_balance": "5.50", "granted_balance": "0.00", "topped_up_balance": "5.50"}]});
        let u = parse_deepseek(&j);
        assert_eq!(u["_credits"]["balance"], 110.0);
        assert_eq!(u["_credits"]["currency"], "CNY");
        assert!(!u.keys().any(|k| !k.starts_with('_')));
        let infos = u["_info"].as_array().unwrap();
        assert!(infos.iter().any(|i| i["value"] == "$5.50"));
        let low = parse_deepseek(&json!({"is_available": false, "balance_infos": []}));
        assert!(low.contains_key("_info"));
    }

    #[test]
    fn moonshot_balance() {
        let j = json!({"code": 0, "status": true, "data": {"available_balance": 49.58894, "voucher_balance": 46.58893, "cash_balance": 3.00001}});
        let u = parse_moonshot(&j, "USD");
        assert_eq!(u["_credits"]["balance"], 49.58894);
        assert_eq!(u["_credits"]["currency"], "USD");
        assert_eq!(u["_info"].as_array().unwrap().len(), 2);
        assert_eq!(parse_moonshot(&j, "CNY")["_credits"]["currency"], "CNY");
    }

    #[test]
    fn entry_ids_are_unique_and_never_auto() {
        let mut list = vec![];
        let id = new_entry_id(&list, "openrouter", "Work key!");
        assert_eq!(id, "work-key");
        list.push(KeyedSource { id: id.clone(), provider: "openrouter".into(), name: "x".into(), key: "k".into() });
        assert_eq!(new_entry_id(&list, "openrouter", "Work key"), "work-key-2");
        assert_eq!(new_entry_id(&list, "deepseek", "Work key"), "work-key");
        assert_eq!(new_entry_id(&[], "zai", "auto"), "key");
        assert_eq!(new_entry_id(&[], "zai", ""), "key");
    }

    #[test]
    fn source_ids_and_removability() {
        assert_eq!(source_id("copilot", AUTO_ID), "key:copilot:auto");
        assert!(!is_removable("key:copilot:auto"));
        assert!(is_removable("key:openrouter:work"));
        assert!(!is_removable("custom:x"));
    }

    #[test]
    fn manual_sources_listed_without_network_and_key_not_in_debug() {
        let mut settings = Settings::default();
        settings.keyed_sources.push(KeyedSource {
            id: "work".into(),
            provider: "openrouter".into(),
            name: "Work".into(),
            key: "sk-or-secret".into(),
        });
        let s = sources(&settings);
        let mine = s.iter().find(|x| x.id == "key:openrouter:work").unwrap();
        assert_eq!(mine.kind, "keyed");
        assert_eq!(mine.label, "OpenRouter \u{00b7} Work");
        assert!(!format!("{:?}", settings.keyed_sources).contains("sk-or-secret"));
        assert!(exists(&settings, "key:openrouter:work"));
        assert!(!exists(&settings, "key:openrouter:other"));
    }

    #[test]
    fn catalog_kinds_are_unique() {
        let mut kinds: Vec<_> = CATALOG.iter().map(|d| d.kind).collect();
        kinds.sort();
        kinds.dedup();
        assert_eq!(kinds.len(), CATALOG.len());
    }
}
