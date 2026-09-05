use crate::i18n::{t, t_fmt};
use crate::settings::Settings;
use crate::types::{ExtraUsageView, PopupBar, PopupProfile, PopupSnapshot, Profile};
use chrono::{DateTime, Datelike, Duration, Local, Timelike, Utc};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

pub const PERIOD_5H: i64 = 5 * 3600;

const NUMBER_WORDS: &[(&str, i32)] = &[
    ("one", 1), ("two", 2), ("three", 3), ("four", 4), ("five", 5), ("six", 6),
    ("seven", 7), ("eight", 8), ("nine", 9), ("ten", 10), ("eleven", 11), ("twelve", 12),
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
            let suffix = if unit == "hour" { "h" } else { "d" };
            let mut label = format!("{n}{suffix}");
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
            } else if unit == "hour" {
                format!("{n}hr")
            } else {
                format!("{n} {unit}")
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
        .filter(|(k, v)| {
            *k != "_plan"
                && v.get("utilization").and_then(Value::as_f64).is_some()
                && v.get("resets_at").is_some()
        })
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
    for key in &settings.tooltip_fields {
        if let Some(entry) = data.get(key).and_then(Value::as_object) {
            if let Some(util) = entry.get("utilization").and_then(Value::as_f64) {
                let short = tooltip_label(key);
                let pct = format!("{util:.0}%");
                let reset = time_until(entry.get("resets_at").and_then(Value::as_str).unwrap_or(""), clock_24h);
                let mut line = format!("{short}: {pct}");
                if !reset.is_empty() {
                    line.push_str(&format!(" · {reset}"));
                }
                lines.push(line);
            }
        }
    }
    lines.join("\n")
}

fn display_plan(raw: &str) -> String {
    let text = raw.trim();
    if text.contains('_') && text.chars().all(|c| c.is_lowercase() || c == '_') {
        title_case_variant(text)
    } else {
        text.to_string()
    }
}

pub fn snapshot_to_popup(
    usage: &Map<String, Value>,
    profile: Option<&Profile>,
    last_success: Option<f64>,
    next_poll: Option<f64>,
    refreshing: bool,
    last_error: Option<&str>,
    settings: &Settings,
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
    let mut bars = Vec::new();
    let clock_24h = settings.time_format == "24h";
    for field in expand_popup_fields(&settings.popup_fields, usage) {
        let Some(entry) = usage.get(&field).and_then(Value::as_object) else { continue };
        let Some(pct) = entry.get("utilization").and_then(Value::as_f64) else { continue };
        let resets_at = entry.get("resets_at").and_then(Value::as_str).unwrap_or("");
        let period = field_period(&field);
        let time_pct = period.and_then(|p| elapsed_pct(resets_at, p));
        let warn = pct >= 100.0 || time_pct.map(|t| pct > t).unwrap_or(false);
        bars.push(PopupBar {
            key: field.clone(),
            label: popup_label(&field),
            pct_text: format!("{pct:.0}%"),
            fill_pct: (pct / 100.0).clamp(0.0, 1.0),
            warn,
            reset_text: if resets_at.is_empty() { String::new() } else { time_until(resets_at, clock_24h) },
            dividers: period.map(|p| divider_positions(resets_at, p)).unwrap_or_default(),
            marker_rel: time_pct.map(|t| (t / 100.0).clamp(0.0, 1.0)),
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
            Some(ExtraUsageView {
                has_limit: true,
                pct_text: format!("{:.0}%", used / limit * 100.0),
                fill_pct: (used / limit).clamp(0.0, 1.0),
                spent_text: t_fmt(
                    "extra_usage_spent",
                    &[
                        ("used", &format_credits(used, currency, places)),
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
        installations: vec![],
        status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_fields() {
        assert_eq!(parse_field_name("five_hour"), Some((5, "hour".into(), None)));
        assert_eq!(parse_field_name("seven_day_sonnet"), Some((7, "day".into(), Some("sonnet".into()))));
        assert_eq!(tooltip_label("five_hour"), "5h");
        assert_eq!(tooltip_label("seven_day_sonnet"), "7d Sonnet");
        assert!(elapsed_pct("", PERIOD_5H).is_none());
    }
}
