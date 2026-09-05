use crate::alerts;
use crate::cache::now;
use crate::formatting::field_period;

use crate::state::AppState;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tauri::AppHandle;

const RESET_BUFFER: i64 = 5;

pub fn align_to_reset(interval: i64, next_reset: Option<f64>, poll_fast: i64) -> (i64, bool) {
    let Some(next_reset) = next_reset else { return (interval, false) };
    if next_reset <= 0.0 {
        return (interval, false);
    }
    let danger = poll_fast - RESET_BUFFER;
    let post = next_reset as i64 + RESET_BUFFER;
    if next_reset <= danger as f64 {
        return (poll_fast, true);
    }
    if post as f64 <= interval as f64 * 1.5 {
        return (post, true);
    }
    if next_reset < (interval + danger) as f64 {
        let pre = next_reset as i64 - danger;
        return (if pre >= poll_fast { pre } else { post }, true);
    }
    (interval, false)
}

pub fn spawn(app: AppHandle, state: Arc<AppState>) {
    std::thread::spawn(move || loop_poll(app, state));
}

pub fn force_update(app: AppHandle, state: Arc<AppState>) {
    std::thread::spawn(move || {
        run_update(&app, &state, true);
    });
}

fn loop_poll(app: AppHandle, state: Arc<AppState>) {
    state.cache.ensure_profile(&state, false);
    let mut force_next = false;
    while *state.running.lock() {
        let token_seen = crate::sources::read_access_token(&state);
        run_update(&app, &state, force_next);
        force_next = false;
        let interval = calculate_interval(&state);
        let mut target = now() + interval as f64;
        *state.next_poll_time.lock() = Some(target);
        let mut last_success_seen = state.cache.last_success_time();
        while *state.running.lock() && now() < target {
            std::thread::sleep(Duration::from_secs(1));
            let current = crate::sources::read_access_token(&state);
            if current.is_some() && current != token_seen {
                force_next = true;
                break;
            }
            if target - now() > interval as f64 + state.settings.lock().poll_fast as f64 {
                target = now() + interval as f64;
                *state.next_poll_time.lock() = Some(target);
            }
            let lst = state.cache.last_success_time();
            if let Some(lst) = lst {
                if last_success_seen.map(|s| lst > s).unwrap_or(true) {
                    last_success_seen = Some(lst);
                    let mut new_target = target.max(lst + interval as f64);
                    if let Some(next_reset) = seconds_until_reset(&state) {
                        let aligned = now() + next_reset + RESET_BUFFER as f64;
                        let poll_fast = state.settings.lock().poll_fast as f64;
                        if new_target > aligned || (now() + next_reset - (poll_fast - RESET_BUFFER as f64) < new_target && new_target < now() + next_reset) {
                            new_target = aligned.max(lst + poll_fast);
                        }
                    }
                    target = new_target;
                    *state.next_poll_time.lock() = Some(target);
                }
            }
            if !state.deferred_notifications.lock().is_empty() && !alerts::user_away(&state) {
                alerts::flush_deferred(&app, &state);
            }
            if alerts::user_away(&state) {
                let mut until = None;
                let settings = state.settings.lock().clone();
                if !settings.on_reset_command.is_empty() {
                    if let Some(next_reset) = seconds_until_reset(&state) {
                        until = Some(now() + next_reset + RESET_BUFFER as f64);
                        *state.idle_reset_pending.lock() = true;
                    } else if *state.idle_reset_pending.lock() {
                        until = Some(now() + settings.poll_interval as f64);
                    }
                }
                wait_activity(&state, until);
                if until.is_some() && alerts::user_away(&state) {
                    break;
                }
                alerts::flush_deferred(&app, &state);
                if let Some(lst) = state.cache.last_success_time() {
                    if let Some(next_reset) = seconds_until_reset(&state) {
                        if next_reset < settings.poll_fast as f64 {
                            target = now() + next_reset + RESET_BUFFER as f64;
                            *state.next_poll_time.lock() = Some(target);
                            continue;
                        }
                    }
                    if now() - lst >= interval as f64 {
                        break;
                    }
                }
            }
        }
    }
}

fn wait_activity(state: &Arc<AppState>, until: Option<f64>) {
    while *state.running.lock() && alerts::user_away(state) {
        if let Some(u) = until {
            if now() >= u {
                break;
            }
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

fn run_update(app: &AppHandle, state: &Arc<AppState>, force: bool) {
    let result = state.cache.update(state, force);
    let Some(data) = result.data else { return };
    *state.last_response.lock() = data.clone();
    crate::tray::refresh(app, state);
    alerts::emit_update(app, state);
    if let Some(tr) = &result.token_refresh {
        if tr.updated && state.settings.lock().notify_claude_update {
            let msg = crate::i18n::t_fmt("notify_update", &[("old", &tr.old_version), ("new", &tr.new_version)]);
            alerts::notify_or_defer(app, state, "claude_update", &msg, &crate::i18n::t("notify_update_title"));
        }
    }
    if data.contains_key("error") {
        crate::tray::refresh(app, state);
        return;
    }
    if result.token.is_some() && result.token != crate::sources::read_access_token(state) {
        return;
    }
    state.cache.ensure_profile(state, force);
    let uuid = state.cache.profile().and_then(|p| {
        if p.account.uuid.is_empty() { None } else { Some(p.account.uuid) }
    });
    let prev = state.prev_account_uuid.lock().clone();
    if prev.is_some() && uuid.is_none() {
        return;
    }
    if let (Some(prev), Some(cur)) = (prev, uuid.clone()) {
        if prev != cur {
            let email = state.cache.profile().map(|p| p.account.email).unwrap_or_default();
            let msg = if email.is_empty() {
                crate::i18n::t("notify_account_switched_title")
            } else {
                crate::i18n::t_fmt("notify_account_switched", &[("email", &email)])
            };
            alerts::notify_or_defer(app, state, "account_switched", &msg, &crate::i18n::t("notify_account_switched_title"));
            state.prev_utilization.lock().clear();
            state.notified_thresholds.lock().clear();
            *state.prev_account_uuid.lock() = Some(cur);
            return;
        }
    }
    *state.prev_account_uuid.lock() = uuid;
    alerts::process(app, state, &data);
    let settings = state.settings.lock().clone();
    let top_key = settings.icon_fields.first().map(|s| s.split(':').next().unwrap_or(s).to_string()).unwrap_or_default();
    let top_pct = data.get(&top_key).and_then(|v| v.get("utilization")).and_then(Value::as_f64).unwrap_or(0.0);
    let prev_top = state.prev_utilization.lock().get(&top_key).copied();
    if let Some(old) = prev_top {
        if top_pct > old {
            *state.fast_polls_remaining.lock() = settings.poll_fast_extra as i32 + 1;
        } else if *state.fast_polls_remaining.lock() > 0 {
            *state.fast_polls_remaining.lock() -= 1;
        }
    }
}

fn calculate_interval(state: &Arc<AppState>) -> i64 {
    let settings = state.settings.lock().clone();
    let data = state.last_response.lock().clone();
    let mut interval = if data.get("rate_limited").and_then(Value::as_bool) == Some(true) {
        let remaining = state.cache.rate_limit_remaining().ceil() as i64;
        if remaining > 0 { remaining.max(settings.poll_interval) } else { settings.poll_interval }
    } else if data.contains_key("error") {
        settings.poll_error
    } else if *state.fast_polls_remaining.lock() > 0 {
        settings.poll_fast
    } else {
        settings.poll_interval
    };
    let (aligned, engaged) = align_to_reset(interval, seconds_until_reset(state), settings.poll_fast);
    interval = aligned;
    if engaged {
        let mut fast = state.fast_polls_remaining.lock();
        *fast = (*fast).max(2);
    }
    interval
}

fn seconds_until_reset(state: &Arc<AppState>) -> Option<f64> {
    let data = state.last_response.lock().clone();
    let mut nearest: Option<f64> = None;
    for (key, value) in &data {
        let Some(obj) = value.as_object() else { continue };
        let Some(resets) = obj.get("resets_at").and_then(Value::as_str) else { continue };
        if resets.is_empty() { continue; }
        let Some(period) = field_period(key) else { continue };
        let _ = period;
        if let Ok(reset) = chrono::DateTime::parse_from_rfc3339(&resets.replace('Z', "+00:00")) {
            let secs = (reset.with_timezone(&chrono::Utc) - chrono::Utc::now()).num_seconds() as f64;
            if secs > 0.0 {
                nearest = Some(nearest.map(|n| n.min(secs)).unwrap_or(secs));
            }
        }
    }
    nearest
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn align_cases() {
        // danger window
        assert_eq!(align_to_reset(180, Some(10.0), 120), (120, true));
        // post
        assert_eq!(align_to_reset(180, Some(100.0), 120).1, true);
        // far
        assert_eq!(align_to_reset(180, Some(10_000.0), 120), (180, false));
    }
}
