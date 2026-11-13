use crate::settings::Settings;
use crate::sources;
use crate::state::AppState;
use crate::types::Profile;
use parking_lot::Mutex;
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct UpdateResult {
    pub data: Option<Map<String, Value>>,
    pub token: Option<String>,
    /// Identity (source id, plus resolved connection for 9Router groups) the data was fetched for.
    pub source_key: Option<String>,
}

impl UpdateResult {
    fn none() -> Self {
        Self { data: None, token: None, source_key: None }
    }
}

pub struct UsageCache {
    lock: Mutex<()>,
    /// Set while a forced update waits for an in-flight one; later forced calls coalesce into it.
    force_waiting: AtomicBool,
    state: Mutex<Inner>,
}

/// Polling/backoff bookkeeping, tracked per source id so one source's 429s or
/// failures never suppress another.
#[derive(Default, Clone)]
struct SourceState {
    last_success_time: Option<f64>,
    consecutive_errors: i32,
    rate_limit_until: f64,
}

struct Inner {
    usage: Map<String, Value>,
    usage_token: Option<String>,
    profile: Option<Profile>,
    profile_token: Option<String>,
    refreshing: bool,
    last_error: Option<String>,
    last_failed_token: Option<String>,
    per_source: HashMap<String, SourceState>,
    /// Source whose state the zero-argument accessors report (the one last polled).
    active: String,
}

/// Largest exponent used for exponential backoff (2^10); keeps the shift far from overflow.
const MAX_BACKOFF_EXP: i32 = 10;

/// Delay after `errors` consecutive failures: `poll_interval * 2^(errors-1)`, capped by `max_backoff`.
fn backoff_delay(poll_interval: i64, max_backoff: i64, errors: i32) -> i64 {
    let exp = (errors.max(1) - 1).clamp(0, MAX_BACKOFF_EXP) as u32;
    poll_interval.max(0).saturating_mul(1i64 << exp).min(max_backoff)
}

impl UsageCache {
    pub fn new() -> Self {
        Self {
            lock: Mutex::new(()),
            force_waiting: AtomicBool::new(false),
            state: Mutex::new(Inner {
                usage: Map::new(),
                usage_token: None,
                profile: None,
                profile_token: None,
                refreshing: false,
                last_error: None,
                last_failed_token: None,
                per_source: HashMap::new(),
                active: String::new(),
            }),
        }
    }

    pub fn usage(&self) -> Map<String, Value> {
        self.state.lock().usage.clone()
    }
    pub fn profile(&self) -> Option<Profile> {
        self.state.lock().profile.clone()
    }
    pub fn last_success_time(&self) -> Option<f64> {
        let inner = self.state.lock();
        inner.per_source.get(&inner.active).and_then(|s| s.last_success_time)
    }
    pub fn rate_limit_remaining(&self) -> f64 {
        let inner = self.state.lock();
        let until = inner.per_source.get(&inner.active).map(|s| s.rate_limit_until).unwrap_or(0.0);
        (until - now()).max(0.0)
    }
    pub fn last_error(&self) -> Option<String> {
        self.state.lock().last_error.clone()
    }
    pub fn refreshing(&self) -> bool {
        self.state.lock().refreshing
    }

    pub fn ensure_profile(&self, state: &AppState, bypass_rate_limit: bool) {
        let target = sources::capture_target(state);
        let current = target.key();
        {
            let inner = self.state.lock();
            if inner.profile.is_some() && inner.profile_token == current {
                return;
            }
            let until = inner.per_source.get(&target.id).map(|s| s.rate_limit_until).unwrap_or(0.0);
            if !bypass_rate_limit && now() < until {
                return;
            }
        }
        let profile = sources::fetch_profile_for(state, &target);
        // The selection moved while fetching: this profile belongs to another source.
        if sources::read_access_token(state) != current {
            return;
        }
        let mut inner = self.state.lock();
        inner.profile = profile;
        inner.profile_token = current;
    }

    pub fn update(&self, state: &AppState, force: bool) -> UpdateResult {
        if let Some(_guard) = self.lock.try_lock() {
            return self.update_locked(state, force);
        }
        if !force {
            return UpdateResult::none();
        }
        // Another update is in flight (usually the background poll). A forced update
        // (popup opened, refresh clicked, source switched) must not be dropped: wait for
        // it, then fetch again unless it already produced fresh data for the current
        // selection. Only one forced caller waits; the rest coalesce into it.
        if self.force_waiting.swap(true, Ordering::SeqCst) {
            return UpdateResult::none();
        }
        let requested_at = now();
        let _guard = self.lock.lock();
        self.force_waiting.store(false, Ordering::SeqCst);
        if self.fresh_since(state, requested_at) {
            return UpdateResult::none();
        }
        self.update_locked(state, true)
    }

    /// True when the currently selected source recorded a success at or after `since`.
    fn fresh_since(&self, state: &AppState, since: f64) -> bool {
        let sid = sources::capture_target(state).id;
        let inner = self.state.lock();
        inner.per_source.get(&sid).and_then(|s| s.last_success_time).is_some_and(|t| t >= since)
    }

    fn update_locked(&self, state: &AppState, force: bool) -> UpdateResult {
        let settings = state.settings.lock().clone();
        // Resolve the source once; everything below (fetch, refresh, bookkeeping,
        // history) is attributed to this target.
        let target = sources::capture_target(state);
        let sid = target.id.clone();
        let token_before = target.key();
        {
            let mut inner = self.state.lock();
            inner.active = sid.clone();
            let n = now();
            let ss = inner.per_source.entry(sid.clone()).or_default();
            if let Some(last) = ss.last_success_time {
                if n < last {
                    ss.last_success_time = Some(n - settings.poll_fast as f64);
                }
            }
            if ss.rate_limit_until - n > settings.max_backoff as f64 {
                ss.rate_limit_until = n + settings.max_backoff as f64;
            }
            if !force {
                if let Some(last) = ss.last_success_time {
                    if n - last < settings.poll_fast as f64 {
                        return UpdateResult::none();
                    }
                }
                if n < ss.rate_limit_until {
                    return UpdateResult::none();
                }
            }
            if let Some(failed) = &inner.last_failed_token {
                if token_before.as_ref() == Some(failed) {
                    return UpdateResult::none();
                }
                inner.last_failed_token = None;
            }
            inner.refreshing = true;
        }
        let mut data = sources::fetch_usage_for(state, &target);
        if data.contains_key("error") {
            let stale = self.is_stale(state, &token_before);
            self.record_error(&sid, &data, true, !stale);
            if data.get("rate_limited").and_then(Value::as_bool) == Some(true) {
                self.apply_backoff(&sid, &data, &settings);
            }
            if data.get("auth_error").and_then(Value::as_bool) == Some(true) && !stale {
                // OAuth refresh belongs to each provider. Never update installed
                // software as a side effect of a failed usage request.
                data = sources::fetch_usage_for(state, &target);
                if self.is_stale(state, &token_before) {
                    self.state.lock().refreshing = false;
                    return UpdateResult::none();
                }
                if !data.contains_key("error") {
                    self.record_success(&sid, &data, token_before.clone(), true);
                    return UpdateResult { data: Some(data), token: token_before.clone(), source_key: token_before };
                }
                if data.get("auth_error").and_then(Value::as_bool) == Some(true) {
                    self.state.lock().last_failed_token = token_before.clone();
                }
                self.record_error(&sid, &data, false, true);
                if data.get("rate_limited").and_then(Value::as_bool) == Some(true) {
                    self.apply_backoff(&sid, &data, &settings);
                }
            }
            let mut inner = self.state.lock();
            inner.refreshing = false;
            drop(inner);
            if stale || self.is_stale(state, &token_before) {
                return UpdateResult::none();
            }
            return UpdateResult { data: Some(data), token: None, source_key: token_before };
        }
        let stale = self.is_stale(state, &token_before);
        self.record_success(&sid, &data, token_before.clone(), !stale);
        if stale {
            // The selection changed mid-fetch: keep the bookkeeping, drop the data.
            return UpdateResult::none();
        }
        UpdateResult { data: Some(data), token: token_before.clone(), source_key: token_before }
    }

    /// True when the currently selected source no longer matches the fetched target.
    fn is_stale(&self, state: &AppState, fetched: &Option<String>) -> bool {
        sources::read_access_token(state) != *fetched
    }

    fn apply_backoff(&self, sid: &str, data: &Map<String, Value>, settings: &Settings) {
        let mut inner = self.state.lock();
        let ss = inner.per_source.entry(sid.to_string()).or_default();
        let delay = match data.get("retry_after").and_then(Value::as_i64).filter(|v| *v > 0) {
            Some(ra) => ra.max(settings.poll_interval).min(settings.max_backoff),
            None => backoff_delay(settings.poll_interval, settings.max_backoff, ss.consecutive_errors),
        };
        ss.rate_limit_until = now() + delay as f64;
    }

    /// `apply` is false for results that no longer match the selected source: the
    /// per-source counters still advance, the user-visible error does not change.
    fn record_error(&self, sid: &str, data: &Map<String, Value>, count: bool, apply: bool) {
        let mut inner = self.state.lock();
        if count {
            let ss = inner.per_source.entry(sid.to_string()).or_default();
            ss.consecutive_errors = ss.consecutive_errors.saturating_add(1);
        }
        if !apply {
            return;
        }
        let mut error = data.get("error").and_then(Value::as_str).unwrap_or("error").to_string();
        if let Some(msg) = data.get("server_message").and_then(Value::as_str) {
            error.push('\n');
            error.push_str(msg);
        }
        inner.last_error = Some(error);
    }

    fn record_success(&self, sid: &str, data: &Map<String, Value>, token: Option<String>, apply: bool) {
        let mut inner = self.state.lock();
        let ss = inner.per_source.entry(sid.to_string()).or_default();
        ss.consecutive_errors = 0;
        ss.last_success_time = Some(now());
        ss.rate_limit_until = 0.0;
        inner.refreshing = false;
        if !apply {
            return;
        }
        inner.last_error = None;
        inner.last_failed_token = None;
        inner.usage = data.clone();
        inner.usage_token = token;
    }
}

pub fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn now_is_positive() {
        assert!(now() > 0.0);
    }

    #[test]
    fn backoff_never_overflows() {
        assert_eq!(backoff_delay(60, 1_000_000, 0), 60);
        assert_eq!(backoff_delay(60, 1_000_000, 1), 60);
        assert_eq!(backoff_delay(60, 1_000_000, 2), 120);
        assert_eq!(backoff_delay(60, 1_000_000, 4), 480);
        assert_eq!(backoff_delay(60, 1_000_000, 11), 60 * 1024);
        // Absurd error counts and intervals must clamp, not panic or wrap.
        assert_eq!(backoff_delay(60, 1_000_000, i32::MAX), 60 * 1024);
        assert_eq!(backoff_delay(i64::MAX, i64::MAX, i32::MAX), i64::MAX);
        assert_eq!(backoff_delay(60, 300, 50), 300);
        assert!(backoff_delay(60, 300, -5) >= 0);
    }

    fn rate_limited(retry_after: Option<i64>) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("error".into(), json!("429"));
        m.insert("rate_limited".into(), json!(true));
        if let Some(ra) = retry_after {
            m.insert("retry_after".into(), json!(ra));
        }
        m
    }

    fn until(cache: &UsageCache, sid: &str) -> f64 {
        cache.state.lock().per_source.get(sid).map(|s| s.rate_limit_until).unwrap_or(0.0)
    }

    #[test]
    fn rate_limit_is_per_source() {
        let cache = UsageCache::new();
        let settings = Settings::default();
        cache.apply_backoff("claude", &rate_limited(Some(settings.max_backoff)), &settings);
        assert!(until(&cache, "claude") > now());
        assert_eq!(until(&cache, "codex"), 0.0);

        // Errors and successes are isolated too.
        cache.record_error("claude", &rate_limited(None), true, true);
        cache.record_error("claude", &rate_limited(None), true, true);
        assert_eq!(cache.state.lock().per_source["claude"].consecutive_errors, 2);
        cache.record_success("codex", &Map::new(), None, true);
        assert_eq!(cache.state.lock().per_source["claude"].consecutive_errors, 2);
        assert!(until(&cache, "claude") > now());
        assert!(cache.state.lock().per_source["codex"].last_success_time.is_some());
        assert!(cache.state.lock().per_source["claude"].last_success_time.is_none());
    }

    #[test]
    fn stale_results_do_not_touch_visible_state() {
        let cache = UsageCache::new();
        let mut data = Map::new();
        data.insert("five_hour".into(), json!({"utilization": 10.0}));
        cache.record_success("old", &data, Some("old".into()), false);
        assert!(cache.usage().is_empty());
        assert!(cache.state.lock().per_source["old"].last_success_time.is_some());
        cache.record_error("old", &rate_limited(None), true, false);
        assert!(cache.last_error().is_none());
        cache.record_success("new", &data, Some("new".into()), true);
        assert_eq!(cache.usage().len(), 1);
    }
}
