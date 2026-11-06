pub mod antigravity;
pub mod claude;
pub mod claude_web;
pub mod codex;
pub mod custom;
pub mod grok;
pub mod keyed;
pub mod ninerouter;
pub mod omp;

use crate::settings::Settings;
use crate::state::AppState;
use crate::types::{PopupGroup, Profile, Source, SourceToggle};
use regex::Regex;
use serde_json::{json, Map, Value};
use parking_lot::Mutex;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

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

// ---- discovery / availability caches ---------------------------------------------------------

/// How long a cheap availability probe (credential file, PATH lookup, ...) stays valid.
const AVAIL_TTL: Duration = Duration::from_secs(15);
/// How long a full discovery result stays valid.
const DISCOVERY_TTL: Duration = Duration::from_secs(5);

/// Bumped by `invalidate_caches`; results computed under an older value are never stored.
static GENERATION: AtomicU64 = AtomicU64::new(0);
static AVAIL: LazyLock<Mutex<HashMap<String, (Instant, bool)>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

struct Discovery {
    at: Instant,
    fingerprint: String,
    items: Vec<Source>,
}

static DISCOVERY: LazyLock<Mutex<Option<Discovery>>> = LazyLock::new(|| Mutex::new(None));
/// Single-flight guard so concurrent callers share one (possibly slow) discovery run.
static DISCOVERY_FLIGHT: Mutex<()> = Mutex::new(());

/// Drop every cached discovery/availability result. Call after any settings or
/// sign-in/out change that can alter which sources exist.
pub fn invalidate_caches() {
    GENERATION.fetch_add(1, Ordering::SeqCst);
    AVAIL.lock().clear();
    *DISCOVERY.lock() = None;
}

/// Memoise a cheap availability probe for `AVAIL_TTL`. `f` runs without any lock held.
pub fn cached_bool(key: &str, f: impl FnOnce() -> bool) -> bool {
    if let Some((at, value)) = AVAIL.lock().get(key) {
        if at.elapsed() < AVAIL_TTL {
            return *value;
        }
    }
    let generation = GENERATION.load(Ordering::SeqCst);
    let value = f();
    if GENERATION.load(Ordering::SeqCst) == generation {
        AVAIL.lock().insert(key.to_string(), (Instant::now(), value));
    }
    value
}

/// The parts of the settings (plus the cheap sign-in marker) discovery depends on.
fn discovery_fingerprint(settings: &Settings) -> String {
    let mut fp = format!("{}|{}", settings.ninerouter_url, claude_web::has_login());
    for custom in &settings.custom_sources {
        fp.push_str(&format!("|c:{}:{}", custom.id, custom.name));
    }
    for keyed in &settings.keyed_sources {
        fp.push_str(&format!("|k:{}:{}:{}", keyed.provider, keyed.id, keyed.name));
    }
    fp
}

fn discovery_lookup(fingerprint: &str) -> Option<Vec<Source>> {
    let cache = DISCOVERY.lock();
    let hit = cache.as_ref()?;
    (hit.fingerprint == fingerprint && hit.at.elapsed() < DISCOVERY_TTL).then(|| hit.items.clone())
}

pub fn list_discovered_sources(settings: &Settings) -> Vec<Source> {
    let fingerprint = discovery_fingerprint(settings);
    if let Some(items) = discovery_lookup(&fingerprint) {
        return items;
    }
    let _flight = DISCOVERY_FLIGHT.lock();
    if let Some(items) = discovery_lookup(&fingerprint) {
        return items;
    }
    let generation = GENERATION.load(Ordering::SeqCst);
    let items = discover_sources(settings);
    if GENERATION.load(Ordering::SeqCst) == generation {
        *DISCOVERY.lock() = Some(Discovery { at: Instant::now(), fingerprint, items: items.clone() });
    }
    items
}

fn claude_available() -> bool {
    cached_bool("avail:claude", || claude::token().is_some())
}
fn codex_available() -> bool {
    cached_bool("avail:codex", || codex::token().is_some())
}
fn antigravity_available() -> bool {
    cached_bool("avail:antigravity", antigravity::has_login)
}
fn grok_available() -> bool {
    cached_bool("avail:grok", grok::has_account)
}
fn omp_installed() -> bool {
    cached_bool("avail:omp", omp::installed)
}

fn discover_sources(settings: &Settings) -> Vec<Source> {
    let mut items = Vec::new();
    if claude_available() {
        items.push(Source { id: "claude".into(), kind: "claude".into(), label: "Claude".into() });
    }
    if claude_web::has_login() {
        items.push(Source { id: claude_web::SOURCE_ID.into(), kind: "claude-web".into(), label: "Claude (web)".into() });
    }
    if codex_available() {
        items.push(Source { id: "codex".into(), kind: "codex".into(), label: "Codex".into() });
    }
    if antigravity_available() {
        items.push(Source { id: "antigravity".into(), kind: "antigravity".into(), label: "Antigravity".into() });
    }
    if grok_available() {
        items.push(Source { id: "grok".into(), kind: "grok".into(), label: "Grok".into() });
    }
    if omp_installed() {
        let omp_sources = omp::sources();
        if omp_sources.is_empty() {
            items.push(Source { id: "omp".into(), kind: "omp".into(), label: "OMP".into() });
        } else {
            for acc in omp_sources {
                items.push(Source {
                    id: format!("omp:{}", acc.id),
                    kind: "omp".into(),
                    label: format!("OMP \u{00b7} {}", acc.label),
                });
            }
        }
    }
    for (provider, conns) in ninerouter_groups(settings) {
        if conns.len() == 1 {
            let conn = &conns[0];
            items.push(Source {
                id: format!("9r:{}", conn.id),
                kind: "9router".into(),
                label: format!(
                    "9Router \u{00b7} {}: {}",
                    if conn.provider.is_empty() { "provider" } else { &conn.provider },
                    conn.display_name()
                ),
            });
        } else {
            items.push(Source {
                id: format!("9rg:{provider}"),
                kind: "9router".into(),
                label: format!("9Router \u{00b7} {provider} ({})", conns.len()),
            });
        }
    }
    items.extend(keyed::sources(settings));
    for custom in &settings.custom_sources {
        items.push(Source {
            id: format!("custom:{}", custom.id),
            kind: "custom".into(),
            label: format!("Custom \u{00b7} {}", custom.name),
        });
    }
    items
}

pub fn list_sources(settings: &Settings) -> Vec<Source> {
    list_discovered_sources(settings)
        .into_iter()
        .filter(|source| !settings.is_hidden(&source.id))
        .collect()
}

/// 9Router connections bucketed by provider, preserving the connections()
/// display order. Providers with one connection stay a plain entry; with
/// several they become a single group entry.
fn ninerouter_groups(settings: &Settings) -> Vec<(String, Vec<ninerouter::Conn>)> {
    group_by_provider(ninerouter::connections(settings))
}

fn group_by_provider(conns: Vec<ninerouter::Conn>) -> Vec<(String, Vec<ninerouter::Conn>)> {
    let mut groups: Vec<(String, Vec<ninerouter::Conn>)> = Vec::new();
    for conn in conns {
        match groups.iter_mut().find(|(p, _)| *p == conn.provider) {
            Some((_, v)) => v.push(conn),
            None => groups.push((conn.provider.clone(), vec![conn])),
        }
    }
    groups
}

pub fn source_toggles(settings: &Settings) -> Vec<SourceToggle> {
    list_discovered_sources(settings)
        .into_iter()
        .map(|source| {
            let detail = if source.kind == "keyed" {
                keyed::detail(&source.id)
            } else if source.kind == "custom" {
                let slug = source.id.strip_prefix("custom:").unwrap_or(&source.id);
                settings
                    .custom_sources
                    .iter()
                    .find(|custom| custom.id == slug)
                    .map(|custom| custom.url.clone())
                    .unwrap_or_default()
            } else {
                String::new()
            };
            SourceToggle {
                visible: !settings.is_hidden(&source.id),
                removable: source.kind == "custom" || (source.kind == "keyed" && keyed::is_removable(&source.id)),
                id: source.id,
                kind: source.kind,
                label: source.label,
                detail,
            }
        })
        .collect()
}

/// Recent-consumption signals for every field of the current source.
pub fn recent_rates(state: &AppState) -> std::collections::HashMap<String, crate::history::RecentRate> {
    let source = read_access_token(state).unwrap_or_default();
    crate::history::rates_for(&state.usage_history.lock(), &source, crate::history::now_secs())
}

pub fn current_source_id(state: &AppState) -> String {
    let current = state.current_id.lock().clone();
    // Work on a snapshot: availability checks and discovery must never run under the lock.
    let settings = state.settings.lock().clone();
    if let Some(id) = current {
        if !id.is_empty() && !settings.is_hidden(&id) {
            return id;
        }
    }
    let sid = settings.source_id.clone();
    if settings.is_hidden(&sid) {
        return default_source_id(&settings);
    }
    if sid == "claude" && !claude_available() {
        return default_source_id(&settings);
    }
    if sid == claude_web::SOURCE_ID && !claude_web::has_login() {
        return default_source_id(&settings);
    }
    if sid == "codex" && !codex_available() {
        return default_source_id(&settings);
    }
    if sid == "antigravity" && !antigravity_available() {
        return default_source_id(&settings);
    }
    if sid == "grok" && !grok_available() {
        return default_source_id(&settings);
    }
    if sid.starts_with("key:") && !cached_bool(&format!("exists:{sid}"), || keyed::exists(&settings, &sid)) {
        return default_source_id(&settings);
    }
    if (sid == "omp" || sid.starts_with("omp:")) && !cached_bool(&format!("omp:{sid}"), || omp_source_available(&sid)) {
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

pub fn ensure_visible_selection(state: &AppState) -> bool {
    let current = state.current_id.lock().clone();
    // Snapshot first: discovery may spawn a process or hit the network.
    let settings = state.settings.lock().clone();
    let visible = list_sources(&settings);
    let current = current.unwrap_or_else(|| settings.source_id.clone());
    if !current.is_empty() && visible.iter().any(|source| source.id == current) {
        return false;
    }
    let next = visible.into_iter().next().map(|source| source.id).unwrap_or_default();
    select_source(state, next);
    true
}

/// The source a fetch is attributed to, resolved once so usage, profile and
/// history all refer to the same source even if the selection changes mid-fetch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub id: String,
    /// Resolved connection id for grouped 9Router sources (`9rg:`); empty when none resolves.
    pub conn: Option<String>,
}

impl Target {
    /// Identity of the data source: the id, plus the resolved connection for 9Router groups.
    pub fn key(&self) -> Option<String> {
        if self.id.is_empty() {
            return None;
        }
        Some(match &self.conn {
            Some(conn) => format!("{}@{conn}", self.id),
            None => self.id.clone(),
        })
    }
}

pub fn capture_target(state: &AppState) -> Target {
    let id = current_source_id(state);
    let conn = id.strip_prefix("9rg:").map(|provider| {
        let settings = state.settings.lock().clone();
        ninerouter::resolve_group(&settings, provider).map(|c| c.id).unwrap_or_default()
    });
    Target { id, conn }
}

pub fn read_access_token(state: &AppState) -> Option<String> {
    capture_target(state).key()
}

pub fn fetch_usage_for(state: &AppState, target: &Target) -> Map<String, Value> {
    let sid = &target.id;
    if sid.is_empty() {
        return error(&crate::i18n::t("no_token"));
    }
    let (kind, payload) = parse_id(sid);
    match kind.as_str() {
        "claude" => claude::usage(),
        "claude-web" => claude_web::usage(),
        "codex" => codex::usage(),
        "antigravity" => antigravity::usage(),
        "grok" => grok::usage(),
        "omp" => omp::usage(&payload),
        "9router" => ninerouter::usage(&state.settings.lock().clone(), &payload),
        "9router-group" => match target.conn.as_deref().filter(|c| !c.is_empty()) {
            Some(conn) => ninerouter::usage(&state.settings.lock().clone(), conn),
            None => error(&crate::i18n::t("no_token")),
        },
        "custom" => custom::usage(&state.settings.lock().clone(), &payload),
        "keyed" => {
            let settings = state.settings.lock().clone();
            keyed::usage(&settings, &payload)
        }
        _ => error(&crate::i18n::t("no_token")),
    }
}

pub fn fetch_profile_for(state: &AppState, target: &Target) -> Option<Profile> {
    let sid = &target.id;
    if sid.is_empty() {
        return None;
    }
    let (kind, payload) = parse_id(sid);
    match kind.as_str() {
        "claude" => claude::profile(),
        "claude-web" => claude_web::profile(),
        "codex" => codex::profile(),
        "antigravity" => antigravity::profile(),
        "grok" => grok::profile(),
        "omp" => omp::profile(&payload),
        "9router" => ninerouter::profile(&state.settings.lock().clone(), &payload),
        "9router-group" => {
            let conn = target.conn.as_deref().filter(|c| !c.is_empty())?;
            ninerouter::profile(&state.settings.lock().clone(), conn)
        }
        "custom" => custom::profile(&state.settings.lock().clone(), sid, &payload),
        "keyed" => keyed::profile(&state.settings.lock().clone(), sid, &payload),
        _ => None,
    }
}

/// Info for the popup's account switcher when the current source is a
/// 9Router group with more than one connection.
pub fn group_info(state: &AppState) -> Option<PopupGroup> {
    let sid = current_source_id(state);
    let provider = sid.strip_prefix("9rg:")?;
    let settings = state.settings.lock().clone();
    let conns = ninerouter::provider_connections(&settings, provider);
    if conns.len() < 2 {
        return None;
    }
    let selected = ninerouter::resolve_group(&settings, provider);
    let index = selected
        .as_ref()
        .and_then(|c| conns.iter().position(|x| x.id == c.id))
        .unwrap_or(0);
    Some(PopupGroup {
        provider: provider.to_string(),
        index: index as i32 + 1,
        count: conns.len() as i32,
        name: selected.map(|c| c.display_name()).unwrap_or_default(),
    })
}

/// Cycle the selected connection of the currently active 9Router group.
pub fn cycle_group_account(state: &AppState, direction: i32) {
    let sid = current_source_id(state);
    let Some(provider) = sid.strip_prefix("9rg:").map(str::to_string) else { return };
    let settings = state.settings.lock().clone();
    let conns = ninerouter::provider_connections(&settings, &provider);
    if conns.len() < 2 {
        return;
    }
    let current = ninerouter::resolve_group(&settings, &provider);
    let idx = current
        .and_then(|c| conns.iter().position(|x| x.id == c.id))
        .unwrap_or(0);
    let next = ((idx as i32 + direction).rem_euclid(conns.len() as i32)) as usize;
    let conn = conns[next].clone();
    {
        let mut settings = state.settings.lock();
        let _ = settings.set_group_selection(&provider, &conn.id);
    }
    // Manual cycling is not an account switch; skip the notification.
    *state.prev_account_uuid.lock() = Some(conn.id);
}

fn parse_id(source_id: &str) -> (String, String) {
    if let Some(rest) = source_id.strip_prefix("9rg:") {
        ("9router-group".into(), rest.into())
    } else if let Some(rest) = source_id.strip_prefix("9r:") {
        ("9router".into(), rest.into())
    } else if let Some(rest) = source_id.strip_prefix("custom:") {
        ("custom".into(), rest.into())
    } else if let Some(rest) = source_id.strip_prefix("key:") {
        ("keyed".into(), rest.into())
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

pub fn utilization(quota: &Map<String, Value>) -> f64 {
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

    #[test]
    fn hidden_sources_are_omitted() {
        let mut settings = Settings::default();
        settings.custom_sources.push(crate::types::CustomSource {
            id: "mine".into(),
            name: "Mine".into(),
            url: "http://example.com".into(),
            token: String::new(),
            header: "Authorization".into(),
            fields: vec![],
        });
        assert!(list_sources(&settings).iter().any(|s| s.id == "custom:mine"));
        settings.set_source_visible("custom:mine", false);
        assert!(!list_sources(&settings).iter().any(|s| s.id == "custom:mine"));
        let mine = source_toggles(&settings).into_iter().find(|s| s.id == "custom:mine").unwrap();
        assert!(!mine.visible);
        assert!(mine.removable);
        assert_eq!(mine.detail, "http://example.com");
    }

    #[test]
    fn cached_bool_memoises_until_invalidated() {
        let calls = std::cell::Cell::new(0);
        let probe = || {
            calls.set(calls.get() + 1);
            true
        };
        assert!(cached_bool("test:memo", probe));
        assert!(cached_bool("test:memo", probe));
        assert_eq!(calls.get(), 1);
        invalidate_caches();
        assert!(cached_bool("test:memo", probe));
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn target_key_includes_group_connection() {
        let plain = Target { id: "claude".into(), conn: None };
        assert_eq!(plain.key().as_deref(), Some("claude"));
        let group = Target { id: "9rg:codex".into(), conn: Some("c1".into()) };
        assert_eq!(group.key().as_deref(), Some("9rg:codex@c1"));
        assert_ne!(group.key(), Target { id: "9rg:codex".into(), conn: Some("c2".into()) }.key());
        assert_eq!(Target { id: String::new(), conn: None }.key(), None);
    }

    #[test]
    fn groups_connections_by_provider() {
        let conn = |id: &str, provider: &str| ninerouter::Conn {
            id: id.into(),
            provider: provider.into(),
            name: id.into(),
            email: String::new(),
            is_active: true,
        };
        let groups = group_by_provider(vec![
            conn("a", "codex"),
            conn("b", "codex"),
            conn("c", "kimi"),
            conn("d", "codex"),
        ]);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].0, "codex");
        assert_eq!(groups[0].1.len(), 3);
        assert_eq!(groups[1].0, "kimi");
        assert_eq!(groups[1].1.len(), 1);
    }
}
