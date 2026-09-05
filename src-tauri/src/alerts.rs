use crate::command;
use crate::formatting::{elapsed_pct, field_period, format_credits, popup_label};
use crate::i18n::{t, t_fmt};
use crate::platform;
use crate::state::AppState;
use serde_json::{Map, Value};
use std::sync::Arc;
use tauri::Emitter;
use tauri_plugin_notification::NotificationExt;

pub fn process(app: &tauri::AppHandle, state: &Arc<AppState>, data: &Map<String, Value>) {
    let settings = state.settings.lock().clone();
    if data.contains_key("error") {
        return;
    }
    let mut quota_fields = std::collections::HashMap::new();
    for (key, value) in data {
        if key == "extra_usage" {
            continue;
        }
        if let Some(util) = value.get("utilization").and_then(Value::as_f64) {
            quota_fields.insert(key.clone(), util);
        }
    }
    let mut reset_detected = false;
    {
        let prev = state.prev_utilization.lock();
        for (key, pct) in &quota_fields {
            let Some(old) = prev.get(key) else { continue };
            let Some((_, unit, _)) = crate::formatting::parse_field_name(key) else { continue };
            let threshold = if unit == "hour" { 95.0 } else { 98.0 };
            let any_blocking = quota_fields.iter().any(|(other, p)| other != key && *p >= 99.0);
            if *old > threshold && *pct < *old && !any_blocking {
                reset_detected = true;
            }
        }
    }
    if reset_detected {
        notify_or_defer(app, state, "reset", &t("notify_reset"), &t("notify_reset_title"));
    }
    {
        let prev = state.prev_utilization.lock().clone();
        for (key, pct) in &quota_fields {
            if let Some(old) = prev.get(key) {
                if *pct < *old {
                    let entry = data.get(key).and_then(Value::as_object).cloned().unwrap_or_default();
                    let env = command::reset_env(key, *pct, *old, data, &entry);
                    command::run_event_command(&settings.on_reset_command, &env);
                    *state.idle_reset_pending.lock() = false;
                }
            }
        }
    }
    check_thresholds(app, state, data);
    check_extra(app, state, data);
    *state.prev_utilization.lock() = quota_fields;
    if !*state.first_update_done.lock() {
        let mut env = command::quota_env(data);
        env.push(("USAGE_MONITOR_EVENT".into(), "startup".into()));
        command::run_event_command(&settings.on_startup_command, &env);
        *state.first_update_done.lock() = true;
    }
}

fn check_thresholds(app: &tauri::AppHandle, state: &Arc<AppState>, data: &Map<String, Value>) {
    let settings = state.settings.lock().clone();
    for (variant_key, entry) in data {
        if variant_key == "extra_usage" {
            continue;
        }
        let Some(obj) = entry.as_object() else { continue };
        let Some(pct) = obj.get("utilization").and_then(Value::as_f64) else { continue };
        let thresholds = settings.get_alert_thresholds(variant_key);
        if thresholds.is_empty() {
            continue;
        }
        let exceeded: Vec<f64> = thresholds.into_iter().filter(|t| pct >= *t).collect();
        let highest = exceeded.into_iter().fold(0.0_f64, f64::max);
        let last = *state.notified_thresholds.lock().get(variant_key).unwrap_or(&0.0);
        if settings.alert_time_aware && highest > last && highest < settings.alert_time_aware_below {
            if let Some(period) = field_period(variant_key) {
                if let Some(time_pct) = elapsed_pct(obj.get("resets_at").and_then(Value::as_str).unwrap_or(""), period) {
                    if pct <= time_pct {
                        state.notified_thresholds.lock().insert(variant_key.clone(), highest);
                        continue;
                    }
                }
            }
        }
        if highest > last {
            let label = popup_label(variant_key);
            let message = t_fmt("notify_threshold_generic", &[("label", &label), ("pct", &format!("{pct:.0}"))]);
            let title = t("notify_threshold_title");
            notify_or_defer(app, state, &format!("threshold_{variant_key}"), &message, &title);
            if *state.first_update_done.lock() {
                let env = command::threshold_env(variant_key, Some(pct), highest, obj, &title, &message, "", "");
                command::run_event_command(&settings.on_threshold_command, &env);
            }
            state.notified_thresholds.lock().insert(variant_key.clone(), highest);
        } else if highest < last {
            state.notified_thresholds.lock().insert(variant_key.clone(), highest);
        }
    }
}

fn check_extra(app: &tauri::AppHandle, state: &Arc<AppState>, data: &Map<String, Value>) {
    let settings = state.settings.lock().clone();
    let Some(extra) = data.get("extra_usage").and_then(Value::as_object) else { return };
    if extra.get("is_enabled").and_then(Value::as_bool) != Some(true) {
        return;
    }
    let used = extra.get("used_credits").and_then(Value::as_f64).unwrap_or(0.0);
    let currency = extra.get("currency").and_then(Value::as_str);
    let places = extra.get("decimal_places").and_then(Value::as_i64);
    let used_text = format_credits(used, currency, places);
    let limit = extra.get("monthly_limit").and_then(Value::as_f64).unwrap_or(0.0);
    if limit > 0.0 {
        let pct = used / limit * 100.0;
        let thresholds = settings.get_alert_thresholds("extra_usage");
        let highest = thresholds.into_iter().filter(|t| pct >= *t).fold(0.0_f64, f64::max);
        let last = *state.notified_thresholds.lock().get("extra_usage").unwrap_or(&0.0);
        if highest > last {
            let limit_text = format_credits(limit, currency, places);
            let message = t_fmt("notify_threshold_extra_usage", &[("pct", &format!("{pct:.0}")), ("used", &used_text), ("limit", &limit_text)]);
            let title = t("notify_threshold_title");
            notify_or_defer(app, state, "threshold_extra_usage", &message, &title);
            if *state.first_update_done.lock() {
                let env = command::threshold_env(
                    "extra_usage",
                    Some(pct),
                    highest,
                    extra,
                    &title,
                    &message,
                    &used_text,
                    &limit_text,
                );
                command::run_event_command(&settings.on_threshold_command, &env);
            }
            state.notified_thresholds.lock().insert("extra_usage".into(), highest);
        } else if highest < last {
            state.notified_thresholds.lock().insert("extra_usage".into(), highest);
        }
    }
    if !settings.alert_extra_usage_spent.is_empty() {
        let places_n = places.unwrap_or(2) as i32;
        let spent = used / 10f64.powi(places_n);
        let highest = settings.alert_extra_usage_spent.iter().copied().filter(|a| spent >= *a).fold(0.0_f64, f64::max);
        let last = *state.notified_thresholds.lock().get("extra_usage_spent").unwrap_or(&0.0);
        if highest > last {
            let message = t_fmt("notify_threshold_extra_usage_spent", &[("used", &used_text)]);
            let title = t("notify_threshold_title");
            notify_or_defer(app, state, "threshold_extra_usage_spent", &message, &title);
            if *state.first_update_done.lock() {
                let env = command::threshold_env(
                    "extra_usage_spent",
                    None,
                    highest,
                    extra,
                    &title,
                    &message,
                    &used_text,
                    "",
                );
                command::run_event_command(&settings.on_threshold_command, &env);
            }
            state.notified_thresholds.lock().insert("extra_usage_spent".into(), highest);
        } else if highest < last {
            state.notified_thresholds.lock().insert("extra_usage_spent".into(), highest);
        }
    }
}

pub fn notify_or_defer(app: &tauri::AppHandle, state: &Arc<AppState>, category: &str, message: &str, title: &str) {
    if user_away(state) {
        state.deferred_notifications.lock().insert(category.into(), (message.into(), title.into()));
    } else {
        let _ = app.notification().builder().title(title).body(message).show();
    }
}

pub fn flush_deferred(app: &tauri::AppHandle, state: &Arc<AppState>) {
    let pending = std::mem::take(&mut *state.deferred_notifications.lock());
    for (message, title) in pending.into_values() {
        let _ = app.notification().builder().title(title).body(message).show();
    }
}

pub fn user_away(state: &Arc<AppState>) -> bool {
    if platform::is_workstation_locked() {
        return true;
    }
    let pause = state.settings.lock().idle_pause;
    pause > 0 && platform::idle_seconds() >= pause as f64
}

pub fn emit_update(app: &tauri::AppHandle, state: &Arc<AppState>) {
    let settings = state.settings.lock().clone();
    let snap = crate::formatting::snapshot_to_popup(
        &state.cache.usage(),
        state.cache.profile().as_ref(),
        state.cache.last_success_time(),
        *state.next_poll_time.lock(),
        state.cache.refreshing(),
        state.cache.last_error().as_deref(),
        &settings,
    );
    let _ = app.emit("usage://update", snap);
}
