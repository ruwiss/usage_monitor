use crate::history::RecentRate;
use crate::i18n::{t, t_fmt};
use crate::settings::Settings;
use crate::types::{ExtraUsageView, InfoRow, PopupBar, PopupInfo, PopupProfile, PopupSnapshot, Profile};
use chrono::{DateTime, Datelike, Duration, Local, Timelike, Utc};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

pub const PERIOD_5H: i64 = 5 * 3600;

const NUMBER_WORDS: &[(&str, i32)] = &[
    ("one", 1), ("two", 2), ("three", 3), ("four", 4), ("five", 5), ("six", 6),
    ("seven", 7), ("eight", 8), ("nine", 9), ("ten", 10), ("eleven", 11), ("twelve", 12),
    ("thirty", 30),
];

pub fn parse_field_name(field: &str) -> Option<(i32, String, Option<String>)> {
    let mut parts = field.splitn(3, '_');
    let number_word = parts.next()?;
    let unit = parts.next()?;
    let variant = parts.next().map(|s| s.to_string());
    let number = NUMBER_WORDS.iter().find(|(w, _)| *w == number_word)?.1;
    if unit != "hour" && unit != "day" {
        return None;
    }
    Some((number, unit.to_string(), variant))
}

fn title_case_variant(text: &str) -> String {
    text.split('_')
        .map(|w| match w.to_lowercase().as_str() {
            "oauth" => "OAuth".into(),
            "api" => "API".into(),
            "ai" => "AI".into(),
            other => {
                let mut c = other.chars();
                match c.next() {
                    Some(f) => f.to_uppercase().to_string() + c.as_str(),
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn tooltip_label(field: &str) -> String {
    match parse_field_name(field) {
        None => title_case_variant(field),
        Some((n, unit, variant)) => {
            let key = if unit == "hour" { "unit_short_hour" } else { "unit_short_day" };
            let mut label = t_fmt(key, &[("n", &n.to_string())]);
            if let Some(v) = variant {
                label.push(' ');
                label.push_str(&title_case_variant(&v));
            }
            label
        }
    }
}

pub fn popup_label(field: &str) -> String {
    match parse_field_name(field) {
        None => title_case_variant(field),
        Some((n, unit, variant)) => {
            let suffix = if let Some(v) = variant {
                title_case_variant(&v)
            } else {
                let unit_key = if unit == "hour" { "unit_hour" } else { "unit_day" };
                t_fmt(unit_key, &[("n", &n.to_string())])
            };
            let key = if unit == "hour" { "session_label" } else { "weekly_label" };
            t_fmt(key, &[("suffix", &suffix)])
        }
    }
}

pub fn field_period(field: &str) -> Option<i64> {
    let (n, unit, _) = parse_field_name(field)?;
    if unit == "hour" {
        Some(n as i64 * 3600)
    } else if unit == "day" {
        Some(n as i64 * 24 * 3600)
    } else {
        None
    }
}

fn field_sort_key(field: &str) -> (i32, i32, i32, String) {
    match parse_field_name(field) {
        None => (2, 0, 0, field.to_string()),
        Some((n, unit, variant)) => {
            let unit_order = if unit == "hour" { 0 } else { 1 };
            let variant_order = if variant.is_none() { 0 } else { 1 };
            (unit_order, n, variant_order, variant.unwrap_or_default())
        }
    }
}

pub fn expand_popup_fields(popup_fields: &[String], usage_data: &Map<String, Value>) -> Vec<String> {
    let available: HashSet<String> = usage_data
        .iter()
        .filter(|(k, v)| popup_field_available(k, v))
        .map(|(k, _)| k.clone())
        .collect();
    let mut result = Vec::new();
    let mut seen = HashSet::new();
    for field in popup_fields {
        if field == "*" {
            let mut remaining: Vec<_> = available.iter().filter(|f| !seen.contains(*f)).cloned().collect();
            remaining.sort_by_key(|f| field_sort_key(f));
            for f in remaining {
                seen.insert(f.clone());
                result.push(f);
            }
        } else if available.contains(field) && seen.insert(field.clone()) {
            result.push(field.clone());
        }
    }
    result
}

fn popup_field_available(key: &str, value: &Value) -> bool {
    if key.starts_with('_') || key == "extra_usage" {
        return false;
    }
    if value.get("kind").and_then(Value::as_str) == Some("count") {
        return true;
    }
    value.get("utilization").and_then(Value::as_f64).is_some() && value.get("resets_at").is_some()
}

fn invert_display(entry: &Map<String, Value>) -> bool {
    entry.get("invert").and_then(Value::as_bool) == Some(true)
}

pub fn display_pct(entry: &Map<String, Value>, used: f64, show_remaining: bool) -> f64 {
    if show_remaining || invert_display(entry) {
        (100.0 - used).clamp(0.0, 100.0)
    } else {
        used
    }
}

fn format_count(n: f64) -> String {
    if (n - n.round()).abs() < 1e-9 {
        format!("{:.0}", n.round())
    } else {
        format!("{n:.2}")
    }
}

fn entry_label(entry: &Map<String, Value>, field: &str, fallback: impl Fn(&str) -> String) -> String {
    // A plain time-window field (five_hour, seven_day) gets its localized label
    // from the field name; an English label from the API payload would only fight
    // that, so it is ignored. Variant/unknown fields (seven_day_sonnet,
    // gpt-4 requests, ...) keep their payload label.
    if is_plain_window(field) {
        return fallback(field);
    }
    entry
        .get("label")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| fallback(field))
}

/// A time-window field with no model variant, e.g. `five_hour` or `seven_day`.
fn is_plain_window(field: &str) -> bool {
    matches!(parse_field_name(field), Some((_, _, None)))
}

fn parse_iso(s: &str) -> Option<DateTime<Utc>> {
    let cleaned = s.replace('Z', "+00:00");
    DateTime::parse_from_rfc3339(&cleaned)
        .or_else(|_| DateTime::parse_from_str(&cleaned, "%Y-%m-%dT%H:%M:%S%.f%:z"))
        .ok()
        .map(|d| d.with_timezone(&Utc))
}

pub fn elapsed_pct(resets_at: &str, period_seconds: i64) -> Option<f64> {
    if resets_at.is_empty() || period_seconds <= 0 {
        return None;
    }
    let reset = parse_iso(resets_at)?;
    let remaining = (reset - Utc::now()).num_seconds() as f64;
    let elapsed = period_seconds as f64 - remaining;
    Some(elapsed / period_seconds as f64 * 100.0).map(|v| v.clamp(0.0, 100.0))
}

pub fn divider_positions(resets_at: &str, period_seconds: i64) -> Vec<f64> {
    if resets_at.is_empty() || period_seconds <= 0 {
        return vec![];
    }
    let Some(reset_utc) = parse_iso(resets_at) else { return vec![] };
    if period_seconds < 24 * 3600 {
        if period_seconds != PERIOD_5H {
            return vec![];
        }
        return vec![0.2, 0.4, 0.6, 0.8];
    }
    let start_utc = reset_utc - Duration::seconds(period_seconds);
    let start_local = start_utc.with_timezone(&Local);
    let end_local = reset_utc.with_timezone(&Local);
    let mut day = start_local.date_naive() + Duration::days(1);
    let mut positions = Vec::new();
    loop {
        let midnight_naive = day.and_hms_opt(0, 0, 0).unwrap();
        let midnight = midnight_naive.and_local_timezone(Local).single().or_else(|| midnight_naive.and_local_timezone(Local).earliest());
        let Some(midnight) = midnight else { break };
        if midnight >= end_local {
            break;
        }
        let elapsed = (midnight - start_local).num_seconds() as f64;
        let rel = elapsed / period_seconds as f64;
        if rel > 0.003 {
            positions.push(rel);
        }
        day += Duration::days(1);
        if positions.len() > 14 {
            break;
        }
    }
    positions
}

fn format_clock(when: DateTime<Local>, clock_24h: bool) -> String {
    if clock_24h {
        when.format("%H:%M").to_string()
    } else {
        when.format("%I:%M %p").to_string().trim_start_matches('0').to_string()
    }
}

pub fn time_until(iso_str: &str, clock_24h: bool) -> String {
    let Some(reset) = parse_iso(iso_str) else { return String::new() };
    let now = Utc::now();
    let total_seconds = (reset - now).num_seconds();
    if total_seconds < 60 {
        return if total_seconds > -60 { t("resets_imminent") } else { String::new() };
    }
    let mut reset_local = reset.with_timezone(&Local);
    if reset_local.second() >= 30 {
        reset_local += Duration::minutes(1);
    }
    reset_local = reset_local.with_second(0).unwrap_or(reset_local);
    let time_str = format_clock(reset_local, clock_24h);
    let today = Local::now().date_naive();
    let reset_date = reset_local.date_naive();
    let total_min = total_seconds / 60;
    if reset_date == today {
        let duration = if total_min >= 60 {
            t_fmt("duration_hm", &[("h", &(total_min / 60).to_string()), ("m", &(total_min % 60).to_string())])
        } else {
            t_fmt("duration_m", &[("m", &total_min.to_string())])
        };
        return t_fmt("resets_in", &[("duration", &duration), ("clock", &time_str)]);
    }
    if reset_date == today + Duration::days(1) {
        return t_fmt("resets_tomorrow", &[("clock", &time_str)]);
    }
    let idx = reset_local.weekday().num_days_from_monday() as usize;
    let day = crate::i18n::load()
        .get("weekdays")
        .and_then(Value::as_array)
        .and_then(|a| a.get(idx))
        .and_then(Value::as_str)
        .unwrap_or("?")
        .to_string();
    t_fmt("resets_weekday", &[("day", &day), ("clock", &time_str)])
}

pub fn format_credits(minor_units: f64, _currency: Option<&str>, decimal_places: Option<i64>) -> String {
    let places = decimal_places.unwrap_or(2) as i32;
    let amount = minor_units / 10f64.powi(places);
    format!("{:.prec$}", amount, prec = places as usize)
}

pub fn format_tooltip(data: &Map<String, Value>, settings: &Settings) -> String {
    let prefix = if crate::instance::is_default_config_dir() {
        String::new()
    } else {
        crate::instance::effective_config_dir()
            .file_name()
            .map(|n| format!("[{}] ", n.to_string_lossy()))
            .unwrap_or_default()
    };
    if let Some(err) = data.get("error").and_then(Value::as_str) {
        if data.get("auth_error").and_then(Value::as_bool) == Some(true) {
            return format!("{prefix}{}\n{}", t("auth_expired_label"), t("auth_expired_short"));
        }
        let mut error = err.to_string();
        if let Some(msg) = data.get("server_message").and_then(Value::as_str) {
            error.push(' ');
            error.push_str(msg);
        }
        let short: String = error.chars().take(80).collect();
        return format!("{prefix}{}\n{short}", t("error_label"));
    }
    let mut lines = vec![format!("{prefix}{}", t("tooltip_title"))];
    let clock_24h = settings.time_format == "24h";
    let mut shown = HashSet::new();
    for key in &settings.tooltip_fields {
        if push_tooltip_line(&mut lines, data, key, clock_24h, settings.show_remaining, &settings.bar_labels) {
            shown.insert(key.clone());
        }
    }
    if shown.is_empty() {
        let mut fallback: Vec<String> = data
            .iter()
            .filter(|(k, v)| popup_field_available(k, v))
            .map(|(k, _)| k.clone())
            .collect();
        fallback.sort_by_key(|f| field_sort_key(f));
        for key in fallback {
            push_tooltip_line(&mut lines, data, &key, clock_24h, settings.show_remaining, &settings.bar_labels);
        }
    }
    // Nothing measurable: tell the user no quota source is selected (or signed in).
    if lines.len() == 1 && data.is_empty() {
        lines.push(t("warn_no_token"));
        lines.push(t("warn_login"));
    }
    lines.join("\n")
}

fn push_tooltip_line(lines: &mut Vec<String>, data: &Map<String, Value>, key: &str, clock_24h: bool, show_remaining: bool, bar_labels: &std::collections::HashMap<String, String>) -> bool {
    let Some(entry) = data.get(key).and_then(Value::as_object) else { return false };
    let fallback = entry_label(entry, key, tooltip_label);
    let short = bar_labels.get(key).map(String::as_str).unwrap_or(&fallback).to_string();
    let reset = time_until(entry.get("resets_at").and_then(Value::as_str).unwrap_or(""), clock_24h);
    let value = if entry.get("kind").and_then(Value::as_str) == Some("count") {
        let used = crate::sources::as_f64(entry.get("used"));
        let unit = entry.get("unit").and_then(Value::as_str).unwrap_or("requests");
        t_fmt("count_used", &[("used", &format_count(used)), ("unit", unit)])
    } else {
        let Some(util) = entry.get("utilization").and_then(Value::as_f64) else { return false };
        format!("{:.0}%", display_pct(entry, util, show_remaining))
    };
    let mut line = format!("{short}: {value}");
    if !reset.is_empty() {
        line.push_str(&format!(" · {reset}"));
    }
    lines.push(line);
    true
}

fn display_plan(raw: &str) -> String {
    let text = raw.trim();
    if text.contains('_') && text.chars().all(|c| c.is_lowercase() || c == '_') {
        title_case_variant(text)
    } else {
        text.to_string()
    }
}

/// Weight of the recent rate when blending with the whole-window average.
const RECENT_WEIGHT: f64 = 0.7;
/// Pessimistic bound under this many seconds is urgent.
const URGENT_SECS: f64 = 30.0 * 60.0;
/// A range is shown only when the bounds differ by more than this ratio...
const RANGE_MIN_RATIO: f64 = 1.2;
/// ...and by more than this many seconds.
const RANGE_MIN_GAP_SECS: f64 = 10.0 * 60.0;

/// Run-out projection bounds, in seconds from now until the quota reaches 100%.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Forecast {
    /// Pessimistic bound (faster rate); always lands before the window resets.
    pub fast: f64,
    /// Optimistic bound (slower rate); may land after the reset.
    pub slow: f64,
    /// Time at the blended (70% recent / 30% average) rate.
    pub blended: f64,
}

/// Run-out forecast. `recent` carries the recent consumption rate and idle state; without it
/// (history too short) the whole-window average rate is used. `None` when usage is idle, behind
/// pace, or the pessimistic bound is not before the reset.
pub fn forecast_runout(used_pct: f64, elapsed_secs: f64, period_secs: f64, recent: Option<&RecentRate>) -> Option<Forecast> {
    if !(used_pct > 0.0 && used_pct < 100.0) || period_secs <= 0.0 || elapsed_secs <= 0.0 || elapsed_secs >= period_secs {
        return None;
    }
    // Too early in the window the rate is noise.
    if elapsed_secs / period_secs < 0.03 {
        return None;
    }
    if recent.map(|r| r.idle).unwrap_or(false) {
        return None;
    }
    let avg = used_pct / elapsed_secs;
    let (blended, fast, slow) = match recent.and_then(|r| r.rate).filter(|r| *r > 0.0) {
        Some(r) => (RECENT_WEIGHT * r + (1.0 - RECENT_WEIGHT) * avg, r.max(avg), r.min(avg)),
        None => (avg, avg, avg),
    };
    let to_full = |rate: f64| (100.0 - used_pct) / rate;
    let forecast = Forecast { fast: to_full(fast.max(blended)), slow: to_full(slow), blended: to_full(blended) };
    if elapsed_secs + forecast.fast < period_secs {
        Some(forecast)
    } else {
        None
    }
}

/// Average-rate run-out projection (see `forecast_runout`).
pub fn project_runout_secs(used_pct: f64, elapsed_secs: f64, period_secs: f64) -> Option<f64> {
    forecast_runout(used_pct, elapsed_secs, period_secs, None).map(|f| f.blended)
}

/// True when the bounds are far enough apart to be worth showing as a range.
fn is_range(f: &Forecast, remaining_secs: f64) -> bool {
    f.slow < remaining_secs && f.slow > f.fast * RANGE_MIN_RATIO && f.slow - f.fast > RANGE_MIN_GAP_SECS
}

fn forecast_text(f: &Forecast, remaining_secs: f64) -> (String, bool) {
    let urgent = f.fast < URGENT_SECS;
    let text = if is_range(f, remaining_secs) {
        t_fmt("runout_range", &[("from", &format_span(f.fast as i64)), ("to", &format_span(f.slow as i64))])
    } else if f.slow < remaining_secs {
        t_fmt("runout_in", &[("duration", &format_span(f.blended as i64))])
    } else {
        t_fmt("runout_in", &[("duration", &format_span(f.fast as i64))])
    };
    (text, urgent)
}

fn format_span(total_secs: i64) -> String {
    let total_min = (total_secs / 60).max(1);
    if total_min >= 24 * 60 {
        let d = total_min / (24 * 60);
        let h = (total_min % (24 * 60)) / 60;
        t_fmt("duration_dh", &[("d", &d.to_string()), ("h", &h.to_string())])
    } else if total_min >= 60 {
        t_fmt("duration_hm", &[("h", &(total_min / 60).to_string()), ("m", &(total_min % 60).to_string())])
    } else {
        t_fmt("duration_m", &[("m", &total_min.to_string())])
    }
}

/// Returns the forecast text and whether it is urgent (projected run-out within 30 minutes).
fn runout_text(resets_at: &str, period: i64, used_pct: f64, recent: Option<&RecentRate>) -> (String, bool) {
    if resets_at.is_empty() || period <= 0 || used_pct < 10.0 {
        return (String::new(), false);
    }
    let Some(reset) = parse_iso(resets_at) else { return (String::new(), false) };
    let remaining = (reset - Utc::now()).num_seconds() as f64;
    let elapsed = period as f64 - remaining;
    match forecast_runout(used_pct, elapsed, period as f64, recent) {
        Some(f) => forecast_text(&f, remaining),
        None => (String::new(), false),
    }
}

fn value_text(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Number(n) => n.as_f64().map(format_count),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

fn amount_text(v: &Value, currency: &str) -> Option<String> {
    let n = match v {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => s.trim().parse::<f64>().ok()?,
        _ => return None,
    };
    let num = format_count(n);
    Some(match currency.trim() {
        "" => num,
        "USD" | "$" => format!("${num}"),
        c => format!("{num} {c}"),
    })
}

/// Builds the credits/info section from the optional `_credits`, `_info` and `_links` keys.
pub fn build_info(usage: &Map<String, Value>) -> Option<PopupInfo> {
    let mut rows = Vec::new();
    if let Some(c) = usage.get("_credits").and_then(Value::as_object) {
        let currency = c
            .get("currency")
            .or_else(|| c.get("unit"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let custom = c.get("label").and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty());
        let balance_label = custom.map(String::from).unwrap_or_else(|| t("credits_balance"));
        if c.get("unlimited").and_then(Value::as_bool) == Some(true) {
            rows.push(InfoRow { label: balance_label, value: t("credits_unlimited") });
        } else if let Some(v) = c.get("balance").and_then(|v| amount_text(v, currency)) {
            rows.push(InfoRow { label: balance_label, value: v });
        }
        if let Some(v) = c.get("used").and_then(|v| amount_text(v, currency)) {
            rows.push(InfoRow { label: t("credits_used"), value: v });
        }
        if let Some(v) = c.get("limit").and_then(|v| amount_text(v, currency)) {
            rows.push(InfoRow { label: t("credits_limit"), value: v });
        }
    }
    if let Some(items) = usage.get("_info").and_then(Value::as_array) {
        for item in items {
            let label = item.get("label").and_then(Value::as_str).map(str::trim).unwrap_or("");
            let value = item.get("value").and_then(value_text);
            if let (false, Some(value)) = (label.is_empty(), value) {
                rows.push(InfoRow { label: label.to_string(), value });
            }
        }
    }
    let dashboard_url = usage
        .get("_links")
        .and_then(|l| l.get("dashboard_url"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|u| u.starts_with("https://") || u.starts_with("http://"))
        .map(String::from);
    if rows.is_empty() && dashboard_url.is_none() {
        return None;
    }
    Some(PopupInfo { rows, dashboard_url })
}

pub fn snapshot_to_popup(
    usage: &Map<String, Value>,
    profile: Option<&Profile>,
    last_success: Option<f64>,
    next_poll: Option<f64>,
    refreshing: bool,
    last_error: Option<&str>,
    settings: &Settings,
    group: Option<crate::types::PopupGroup>,
) -> PopupSnapshot {
    snapshot_to_popup_with(usage, profile, last_success, next_poll, refreshing, last_error, settings, group, &HashMap::new())
}

/// Like `snapshot_to_popup`, with per-field recent-consumption signals for run-out forecasts.
pub fn snapshot_to_popup_with(
    usage: &Map<String, Value>,
    profile: Option<&Profile>,
    last_success: Option<f64>,
    next_poll: Option<f64>,
    refreshing: bool,
    last_error: Option<&str>,
    settings: &Settings,
    group: Option<crate::types::PopupGroup>,
    rates: &HashMap<String, RecentRate>,
) -> PopupSnapshot {
    let mut popup_profile = profile.map(|p| PopupProfile {
        email: p.account.email.clone(),
        plan: display_plan(&p.organization.organization_type),
    });
    if let Some(plan) = usage.get("_plan").and_then(Value::as_str) {
        if !plan.trim().is_empty() {
            match &mut popup_profile {
                Some(pp) => pp.plan = plan.trim().to_string(),
                None => popup_profile = Some(PopupProfile { email: String::new(), plan: plan.trim().into() }),
            }
        }
    }
    let bar_labels = &settings.bar_labels;
    let mut bars = Vec::new();
    let clock_24h = settings.time_format == "24h";
    let (tray_top, _) = crate::tray::tray_quota_fields(usage);
    for field in expand_popup_fields(&settings.popup_fields, usage) {
        let Some(entry) = usage.get(&field).and_then(Value::as_object) else { continue };
        let label = entry_label(entry, &field, popup_label);
        let label = bar_labels.get(&field).map(String::as_str).unwrap_or(&label).to_string();
        let resets_at = entry.get("resets_at").and_then(Value::as_str).unwrap_or("");
        if entry.get("kind").and_then(Value::as_str) == Some("count") {
            let used = crate::sources::as_f64(entry.get("used"));
            let unit = entry.get("unit").and_then(Value::as_str).unwrap_or("requests");
            bars.push(PopupBar {
                key: field.clone(),
                label,
                pct_text: t_fmt("count_used", &[("used", &format_count(used)), ("unit", unit)]),
                fill_pct: 0.0,
                warn: false,
                reset_text: if resets_at.is_empty() { String::new() } else { time_until(resets_at, clock_24h) },
                dividers: vec![],
                marker_rel: None,
                kind: "text".into(),
                tray_active: false,
                runout_text: String::new(),
                runout_urgent: false,
            });
            continue;
        }
        let Some(pct) = entry.get("utilization").and_then(Value::as_f64) else { continue };
        let shown = display_pct(entry, pct, settings.show_remaining);
        let remaining_style = settings.show_remaining || invert_display(entry);
        let period = field_period(&field);
        let time_pct = period.and_then(|p| elapsed_pct(resets_at, p));
        let warn = if remaining_style {
            shown <= 10.0 || pct >= 100.0
        } else {
            pct >= 90.0
        };
        let (runout_text, runout_urgent) = period.map(|p| runout_text(resets_at, p, pct, rates.get(&field))).unwrap_or_default();
        bars.push(PopupBar {
            key: field.clone(),
            label,
            pct_text: format!("{shown:.0}%"),
            fill_pct: (shown / 100.0).clamp(0.0, 1.0),
            warn,
            reset_text: if resets_at.is_empty() { String::new() } else { time_until(resets_at, clock_24h) },
            dividers: period.map(|p| divider_positions(resets_at, p)).unwrap_or_default(),
            marker_rel: if remaining_style {
                None
            } else {
                time_pct.map(|t| (t / 100.0).clamp(0.0, 1.0))
            },
            kind: "bar".into(),
            tray_active: field == tray_top,
            runout_text,
            runout_urgent,
        });
    }
    let extra = usage.get("extra_usage").and_then(Value::as_object).and_then(|e| {
        if e.get("is_enabled").and_then(Value::as_bool) != Some(true) {
            return None;
        }
        let used = e.get("used_credits").and_then(Value::as_f64)?;
        let limit = e.get("monthly_limit").and_then(Value::as_f64).unwrap_or(0.0);
        let currency = e.get("currency").and_then(Value::as_str);
        let places = e.get("decimal_places").and_then(Value::as_i64);
        if limit > 0.0 {
            let remaining = (limit - used).max(0.0);
            let shown = if settings.show_remaining { remaining } else { used };
            let pct_key = if settings.show_remaining { "extra_usage_remaining" } else { "extra_usage_spent" };
            Some(ExtraUsageView {
                has_limit: true,
                pct_text: format!("{:.0}%", shown / limit * 100.0),
                fill_pct: (shown / limit).clamp(0.0, 1.0),
                spent_text: t_fmt(
                    pct_key,
                    &[
                        ("used", &format_credits(used, currency, places)),
                        ("remaining", &format_credits(remaining, currency, places)),
                        ("limit", &format_credits(limit, currency, places)),
                    ],
                ),
            })
        } else {
            Some(ExtraUsageView {
                has_limit: false,
                pct_text: String::new(),
                fill_pct: 0.0,
                spent_text: t_fmt("extra_usage_spent_no_limit", &[("used", &format_credits(used, currency, places))]),
            })
        }
    });
    let status = if usage.is_empty() {
        if let Some(err) = last_error {
            json!({ "text": err.chars().take(120).collect::<String>(), "is_error": true })
        } else {
            json!({ "text": t("status_refreshing"), "is_error": false, "refreshing": true })
        }
    } else {
        json!({
            "last_success_time": last_success,
            "next_poll_time": next_poll,
            "refreshing": refreshing,
            "error": last_error.map(|e| e.chars().take(120).collect::<String>()),
        })
    };
    PopupSnapshot {
        profile: popup_profile,
        usage: bars,
        extra,
        info: build_info(usage),
        group,
        installations: vec![],
        status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runout_projection() {
        // 50% used after 25% of the window -> full 25% of the period from now.
        let secs = project_runout_secs(50.0, 250.0, 1000.0).unwrap();
        assert!((secs - 250.0).abs() < 1e-6);
        // On or behind pace: nothing.
        assert!(project_runout_secs(25.0, 250.0, 1000.0).is_none());
        assert!(project_runout_secs(10.0, 500.0, 1000.0).is_none());
        // Edge cases.
        assert!(project_runout_secs(0.0, 500.0, 1000.0).is_none());
        assert!(project_runout_secs(100.0, 500.0, 1000.0).is_none());
        assert!(project_runout_secs(50.0, 0.0, 1000.0).is_none());
        assert!(project_runout_secs(50.0, 1000.0, 1000.0).is_none());
        assert!(project_runout_secs(20.0, 10.0, 1000.0).is_none(), "too early in window");
    }

    fn recent(rate: Option<f64>, idle: bool) -> RecentRate {
        RecentRate { rate, idle }
    }

    #[test]
    fn forecast_average_only_fallback() {
        let f = forecast_runout(50.0, 250.0, 1000.0, None).unwrap();
        assert!((f.fast - 250.0).abs() < 1e-6 && (f.slow - 250.0).abs() < 1e-6 && (f.blended - 250.0).abs() < 1e-6);
        // Recent info without a measurable rate and not idle behaves like the average.
        let g = forecast_runout(50.0, 250.0, 1000.0, Some(&recent(None, false))).unwrap();
        assert_eq!(f, g);
    }

    #[test]
    fn forecast_blends_recent_rate() {
        // avg = 0.2 %/s, recent = 0.1 %/s, blended = 0.13 %/s.
        let f = forecast_runout(50.0, 250.0, 1000.0, Some(&recent(Some(0.1), false))).unwrap();
        assert!((f.blended - 50.0 / 0.13).abs() < 1e-6, "{f:?}");
    }

    #[test]
    fn forecast_idle_is_suppressed() {
        assert!(forecast_runout(50.0, 250.0, 1000.0, Some(&recent(None, true))).is_none());
        assert!(forecast_runout(50.0, 250.0, 1000.0, Some(&recent(Some(1.0), true))).is_none());
    }

    #[test]
    fn forecast_burst_has_pessimistic_before_optimistic() {
        // Slow average (0.05 %/s) but a fast recent burst (0.2 %/s).
        let f = forecast_runout(30.0, 600.0, 10_000.0, Some(&recent(Some(0.2), false))).unwrap();
        assert!(f.fast < f.blended && f.blended < f.slow, "{f:?}");
        assert!((f.fast - 70.0 / 0.2).abs() < 1e-6);
        assert!((f.slow - 70.0 / 0.05).abs() < 1e-6);
        // Average alone would not run out before the reset, the burst does.
        assert!(forecast_runout(30.0, 6000.0, 10_000.0, None).is_none());
        assert!(forecast_runout(30.0, 6000.0, 10_000.0, Some(&recent(Some(0.1), false))).is_some());
    }

    #[test]
    fn forecast_range_vs_single_formatting() {
        // Wide gap -> range with the en dash.
        let wide = Forecast { fast: 5400.0, slow: 7800.0, blended: 6000.0 };
        let (text, urgent) = forecast_text(&wide, 20_000.0);
        assert!(text.contains('\u{2013}'), "{text}");
        assert!(!urgent);
        // Gap under 20%.
        let close = Forecast { fast: 5400.0, slow: 6000.0, blended: 5600.0 };
        assert!(!forecast_text(&close, 20_000.0).0.contains('\u{2013}'));
        // Ratio large but under 10 minutes apart.
        let tiny = Forecast { fast: 120.0, slow: 600.0, blended: 300.0 };
        assert!(!is_range(&tiny, 20_000.0));
        // Optimistic bound after the reset: single value from the pessimistic bound.
        let late = Forecast { fast: 5400.0, slow: 30_000.0, blended: 8000.0 };
        assert!(!is_range(&late, 20_000.0));
        assert_eq!(forecast_text(&late, 20_000.0).0, t_fmt("runout_in", &[("duration", &format_span(5400))]));
        // Urgent follows the pessimistic bound.
        let urgent_f = Forecast { fast: 600.0, slow: 5000.0, blended: 1500.0 };
        assert!(forecast_text(&urgent_f, 20_000.0).1);
    }

    #[test]
    fn info_section_and_underscore_keys() {
        let reset = (Utc::now() + Duration::hours(3)).to_rfc3339();
        let mut usage = Map::new();
        usage.insert("five_hour".into(), json!({ "utilization": 80.0, "resets_at": reset }));
        usage.insert("_credits".into(), json!({ "balance": "12.5", "currency": "USD" }));
        usage.insert("_info".into(), json!([{ "label": "Region", "value": "eu" }]));
        usage.insert("_links".into(), json!({ "dashboard_url": "https://example.com/x" }));
        let snap = snapshot_to_popup(&usage, None, None, None, false, None, &Settings::default(), None);
        assert_eq!(snap.usage.len(), 1, "underscore keys must not become bars");
        let info = snap.info.unwrap();
        assert_eq!(info.rows.len(), 2);
        assert_eq!(info.rows[0].value, "$12.50");
        assert_eq!(info.dashboard_url.as_deref(), Some("https://example.com/x"));
        assert!(build_info(&Map::new()).is_none());
    }

    #[test]
    fn parse_fields() {
        assert_eq!(parse_field_name("five_hour"), Some((5, "hour".into(), None)));
        assert_eq!(parse_field_name("seven_day_sonnet"), Some((7, "day".into(), Some("sonnet".into()))));
        assert_eq!(parse_field_name("thirty_day"), Some((30, "day".into(), None)));
        // Labels are localized, so compare against the active translations.
        assert_eq!(tooltip_label("five_hour"), t_fmt("unit_short_hour", &[("n", "5")]));
        let week = t_fmt("unit_short_day", &[("n", "7")]);
        assert_eq!(tooltip_label("seven_day_sonnet"), format!("{week} Sonnet"));
        assert!(elapsed_pct("", PERIOD_5H).is_none());
    }

    #[test]
    fn plain_windows_ignore_payload_label() {
        // A bare time window uses the localized label, not the API's English one.
        assert!(is_plain_window("five_hour"));
        assert!(is_plain_window("seven_day"));
        assert!(!is_plain_window("seven_day_sonnet"));
        assert!(!is_plain_window("gpt_4_requests"));

        let mut plain = Map::new();
        plain.insert("label".into(), json!("Session (5hr)"));
        assert_eq!(entry_label(&plain, "five_hour", popup_label), popup_label("five_hour"));

        // A variant or unknown field keeps its payload label.
        let mut variant = Map::new();
        variant.insert("label".into(), json!("SuperGrok Weekly Credits"));
        assert_eq!(entry_label(&variant, "seven_day_supergrok", popup_label), "SuperGrok Weekly Credits");
    }

    #[test]
    fn plain_window_unit_words_use_templates() {
        // The number is substituted into the localized unit template, not hard-coded.
        let hour = popup_label("five_hour");
        assert!(hour.contains('5'), "{hour}");
        let day = popup_label("seven_day");
        assert!(day.contains('7'), "{day}");
    }

    #[test]
    fn invert_remaining_and_count_text() {
        let mut usage = Map::new();
        usage.insert("seven_day_supergrok".into(), json!({
            "utilization": 69.0,
            "resets_at": "",
            "label": "SuperGrok Weekly Credits",
            "invert": true,
        }));
        usage.insert("thirty_day_gpt_4_requests".into(), json!({
            "kind": "count",
            "used": 0,
            "unit": "requests",
            "label": "gpt-4 requests",
            "resets_at": "",
        }));
        let settings = Settings::default();
        let snap = snapshot_to_popup(&usage, None, None, None, false, None, &settings, None);
        let grok = snap.usage.iter().find(|b| b.key == "seven_day_supergrok").unwrap();
        assert_eq!(grok.pct_text, "31%");
        assert!((grok.fill_pct - 0.31).abs() < 0.001);
        assert_eq!(grok.kind, "bar");
        assert!(grok.marker_rel.is_none());
        let req = snap.usage.iter().find(|b| b.key == "thirty_day_gpt_4_requests").unwrap();
        assert_eq!(req.kind, "text");
        assert_eq!(req.pct_text, t_fmt("count_used", &[("used", "0"), ("unit", "requests")]));
        assert_eq!(req.label, "gpt-4 requests");
    }

    #[test]
    fn bar_label_override_applies_to_popup_and_tooltip() {
        let mut usage = Map::new();
        usage.insert("five_hour".into(), json!({
            "utilization": 40.0,
            "resets_at": "",
            "label": "Session",
        }));
        let mut settings = Settings::default();
        settings.bar_labels.insert("five_hour".into(), "Oturum".into());
        let snap = snapshot_to_popup(&usage, None, None, None, false, None, &settings, None);
        let bar = snap.usage.iter().find(|b| b.key == "five_hour").unwrap();
        assert_eq!(bar.label, "Oturum");
        let tip = format_tooltip(&usage, &settings);
        assert!(tip.contains("Oturum: 40%"), "{tip}");
    }

    #[test]
    fn show_remaining_flips_used_quota() {
        let mut usage = Map::new();
        usage.insert("five_hour".into(), json!({
            "utilization": 40.0,
            "resets_at": "",
            "label": "Session",
        }));
        let mut settings = Settings::default();
        settings.show_remaining = true;
        let snap = snapshot_to_popup(&usage, None, None, None, false, None, &settings, None);
        let bar = snap.usage.iter().find(|b| b.key == "five_hour").unwrap();
        assert_eq!(bar.pct_text, "60%");
        assert!((bar.fill_pct - 0.60).abs() < 0.001);
        assert!(bar.marker_rel.is_none());
    }

    #[test]
    fn show_remaining_keeps_pace_warn() {
        let reset = (Utc::now() + Duration::hours(1)).to_rfc3339();
        let mut usage = Map::new();
        usage.insert("five_hour".into(), json!({
            "utilization": 90.0,
            "resets_at": reset,
            "label": "Session",
        }));
        let mut settings = Settings::default();
        settings.show_remaining = true;
        let snap = snapshot_to_popup(&usage, None, None, None, false, None, &settings, None);
        let bar = snap.usage.iter().find(|b| b.key == "five_hour").unwrap();
        assert_eq!(bar.pct_text, "10%");
        assert!(bar.warn);
        assert!(bar.marker_rel.is_none());
    }

    #[test]
    fn show_remaining_warns_when_low() {
        let reset = (Utc::now() + Duration::hours(1)).to_rfc3339();
        let mut usage = Map::new();
        usage.insert("seven_day".into(), json!({
            "utilization": 90.0,
            "resets_at": reset,
            "label": "Weekly",
        }));
        let mut settings = Settings::default();
        settings.show_remaining = true;
        let snap = snapshot_to_popup(&usage, None, None, None, false, None, &settings, None);
        let bar = snap.usage.iter().find(|b| b.key == "seven_day").unwrap();
        assert_eq!(bar.pct_text, "10%");
        assert!(bar.warn);
        assert!(bar.marker_rel.is_none());
    }

    #[test]
    fn remaining_high_does_not_warn() {
        let reset = (Utc::now() + Duration::hours(5)).to_rfc3339();
        let mut usage = Map::new();
        usage.insert("five_hour".into(), json!({
            "utilization": 1.0,
            "resets_at": reset,
            "label": "Session",
        }));
        let mut settings = Settings::default();
        settings.show_remaining = true;
        let snap = snapshot_to_popup(&usage, None, None, None, false, None, &settings, None);
        let bar = snap.usage.iter().find(|b| b.key == "five_hour").unwrap();
        assert_eq!(bar.pct_text, "99%");
        assert!(!bar.warn);
    }

    #[test]
    fn used_high_warns_when_low_remaining() {
        let reset = (Utc::now() + Duration::hours(1)).to_rfc3339();
        let mut usage = Map::new();
        usage.insert("seven_day".into(), json!({
            "utilization": 90.0,
            "resets_at": reset,
            "label": "Weekly",
        }));
        let settings = Settings::default();
        let snap = snapshot_to_popup(&usage, None, None, None, false, None, &settings, None);
        let bar = snap.usage.iter().find(|b| b.key == "seven_day").unwrap();
        assert_eq!(bar.pct_text, "90%");
        assert!(bar.warn);
    }
}

