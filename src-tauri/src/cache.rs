use crate::settings::Settings;
use crate::sources;
use crate::state::AppState;
use crate::types::{Profile, RefreshResult};
use parking_lot::Mutex;
use serde_json::{Map, Value};
use std::time::{SystemTime, UNIX_EPOCH};

pub struct UpdateResult {
    pub data: Option<Map<String, Value>>,
    pub token_refresh: Option<RefreshResult>,
    pub token: Option<String>,
}

pub struct UsageCache {
    lock: Mutex<()>,
    state: Mutex<Inner>,
}

struct Inner {
    usage: Map<String, Value>,
    usage_token: Option<String>,
    profile: Option<Profile>,
    profile_token: Option<String>,
    last_success_time: Option<f64>,
    refreshing: bool,
    last_error: Option<String>,
    version: u64,
    consecutive_errors: i32,
    last_failed_token: Option<String>,
    rate_limit_until: f64,
}

impl UsageCache {
    pub fn new() -> Self {
        Self {
            lock: Mutex::new(()),
            state: Mutex::new(Inner {
                usage: Map::new(),
                usage_token: None,
                profile: None,
                profile_token: None,
                last_success_time: None,
                refreshing: false,
                last_error: None,
                version: 0,
                consecutive_errors: 0,
                last_failed_token: None,
                rate_limit_until: 0.0,
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
        self.state.lock().last_success_time
    }
    pub fn rate_limit_remaining(&self) -> f64 {
        (self.state.lock().rate_limit_until - now()).max(0.0)
    }
    pub fn last_error(&self) -> Option<String> {
        self.state.lock().last_error.clone()
    }
    pub fn refreshing(&self) -> bool {
        self.state.lock().refreshing
    }
    pub fn version(&self) -> u64 {
        self.state.lock().version
    }

    pub fn ensure_profile(&self, state: &AppState, bypass_rate_limit: bool) {
        let current = sources::read_access_token(state);
        {
            let inner = self.state.lock();
            if inner.profile.is_some() && inner.profile_token == current {
                return;
            }
            if !bypass_rate_limit && now() < inner.rate_limit_until {
                return;
            }
        }
        let profile = sources::fetch_profile(state);
        let mut inner = self.state.lock();
        inner.profile = profile;
        inner.profile_token = current;
        inner.version += 1;
    }

    pub fn update(&self, state: &AppState, force: bool) -> UpdateResult {
        let Some(_guard) = self.lock.try_lock() else {
            return UpdateResult { data: None, token_refresh: None, token: None };
        };
        self.update_locked(state, force)
    }

    fn update_locked(&self, state: &AppState, force: bool) -> UpdateResult {
        let settings = state.settings.lock().clone();
        {
            let mut inner = self.state.lock();
            let n = now();
            if let Some(last) = inner.last_success_time {
                if n < last {
                    inner.last_success_time = Some(n - settings.poll_fast as f64);
                }
            }
            if inner.rate_limit_until - n > settings.max_backoff as f64 {
                inner.rate_limit_until = n + settings.max_backoff as f64;
            }
            if !force {
                if let Some(last) = inner.last_success_time {
                    if n - last < settings.poll_fast as f64 {
                        return UpdateResult { data: None, token_refresh: None, token: None };
                    }
                }
                if n < inner.rate_limit_until {
                    return UpdateResult { data: None, token_refresh: None, token: None };
                }
            }
            if let Some(failed) = &inner.last_failed_token {
                if sources::read_access_token(state).as_ref() == Some(failed) {
                    return UpdateResult { data: None, token_refresh: None, token: None };
                }
                inner.last_failed_token = None;
            }
            inner.refreshing = true;
            inner.version += 1;
        }
        let token_before = sources::read_access_token(state);
        let mut data = sources::fetch_usage(state);
        if data.contains_key("error") {
            self.record_error(&data, true);
            if data.get("rate_limited").and_then(Value::as_bool) == Some(true) {
                self.apply_backoff(&data, &settings);
            }
            let mut token_refresh = None;
            if data.get("auth_error").and_then(Value::as_bool) == Some(true) {
                let (tr, retry) = self.try_refresh(state, token_before.clone());
                token_refresh = tr;
                if token_refresh.is_some() && self.last_error().is_none() {
                    return UpdateResult { data: Some(self.usage()), token_refresh, token: self.state.lock().usage_token.clone() };
                }
                if token_refresh.is_none() {
                    self.state.lock().last_failed_token = token_before.clone();
                }
                if let Some(retry) = retry {
                    data = retry;
                }
            }
            let mut inner = self.state.lock();
            inner.refreshing = false;
            inner.version += 1;
            return UpdateResult { data: Some(data), token_refresh, token: None };
        }
        self.record_success(&data, token_before.clone());
        UpdateResult { data: Some(data), token_refresh: None, token: token_before }
    }

    fn try_refresh(&self, state: &AppState, token_before: Option<String>) -> (Option<RefreshResult>, Option<Map<String, Value>>) {
        let sid = crate::sources::current_source_id(state);
        let result = if sid == "claude" {
            crate::claude_cli::refresh_token()
        } else {
            RefreshResult { success: true, ..Default::default() }
        };
        if sid == "claude" && !result.success {
            let data = sources::fetch_usage(state);
            return (Some(result), Some(data));
        }
        let current = sources::read_access_token(state);
        let data = sources::fetch_usage(state);
        if !data.contains_key("error") {
            self.record_success(&data, current);
            return (Some(result), Some(data));
        }
        self.record_error(&data, false);
        if data.get("rate_limited").and_then(Value::as_bool) == Some(true) {
            self.apply_backoff(&data, &state.settings.lock());
        }
        if current == token_before || current.is_none() {
            return (if result.updated { Some(result) } else { None }, Some(data));
        }
        (Some(result), Some(data))
    }

    fn apply_backoff(&self, data: &Map<String, Value>, settings: &Settings) {
        let delay = match data.get("retry_after").and_then(Value::as_i64).filter(|v| *v > 0) {
            Some(ra) => ra.max(settings.poll_interval).min(settings.max_backoff),
            None => {
                let errors = self.state.lock().consecutive_errors.max(1);
                (settings.poll_interval * 2i64.pow((errors - 1) as u32)).min(settings.max_backoff)
            }
        };
        self.state.lock().rate_limit_until = now() + delay as f64;
    }

    fn record_error(&self, data: &Map<String, Value>, count: bool) {
        let mut inner = self.state.lock();
        if count {
            inner.consecutive_errors += 1;
        }
        let mut error = data.get("error").and_then(Value::as_str).unwrap_or("error").to_string();
        if let Some(msg) = data.get("server_message").and_then(Value::as_str) {
            error.push('\n');
            error.push_str(msg);
        }
        inner.last_error = Some(error);
    }

    fn record_success(&self, data: &Map<String, Value>, token: Option<String>) {
        let mut inner = self.state.lock();
        inner.consecutive_errors = 0;
        inner.last_error = None;
        inner.last_success_time = Some(now());
        inner.rate_limit_until = 0.0;
        inner.last_failed_token = None;
        inner.usage = data.clone();
        inner.usage_token = token;
        inner.refreshing = false;
        inner.version += 1;
    }
}

pub fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_is_positive() {
        assert!(now() > 0.0);
    }
}
