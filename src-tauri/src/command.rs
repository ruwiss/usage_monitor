use crate::formatting::format_credits;
use crate::i18n::{t, t_fmt};
use crate::platform::no_window;
use crate::state::AppState;
use chrono::{Duration, Utc};
use serde_json::{Map, Value};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Instant;

const STARTUP_FAILURE_WINDOW: f64 = 5.0;

pub fn run_event_command(commands: &[String], env_vars: &[(String, String)]) {
    spawn_event_command(commands, env_vars, false, true);
}

pub fn run_event_command_captured(
    commands: &[String],
    env_vars: &[(String, String)],
    report_late_failures: bool,
) {
    spawn_event_command(commands, env_vars, true, report_late_failures);
}

fn spawn_event_command(
    commands: &[String],
    env_vars: &[(String, String)],
    capture_output: bool,
    report_late_failures: bool,
) {
    if commands.is_empty() {
        return;
    }
    let cwd = crate::platform::event_command_cwd();
    for command in commands {
        let command = command.clone();
        let env_vars = env_vars.to_vec();
        let cwd = cwd.clone();
        thread::spawn(move || {
            let mut cmd = shell_command(&command);
            cmd.current_dir(&cwd).stdin(Stdio::null());
            if capture_output {
                cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
            } else {
                cmd.stdout(Stdio::null()).stderr(Stdio::null());
            }
            cmd.env("USAGE_MONITOR_VERSION", env!("CARGO_PKG_VERSION"));
            for (k, v) in &env_vars {
                cmd.env(k, v);
            }
            no_window(&mut cmd);
            if !capture_output {
                let _ = cmd.spawn();
                return;
            }
            let started = Instant::now();
            match cmd.output() {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    eprintln!("[event command] {command}");
                    eprintln!("  exit code: {:?}", out.status.code());
                    eprintln!(
                        "  stdout:\n{}",
                        if stdout.trim().is_empty() {
                            "    (empty)"
                        } else {
                            stdout.trim_end()
                        }
                    );
                    eprintln!(
                        "  stderr:\n{}",
                        if stderr.trim().is_empty() {
                            "    (empty)"
                        } else {
                            stderr.trim_end()
                        }
                    );
                    if out.status.success() {
                        return;
                    }
                    let runtime = started.elapsed().as_secs_f64();
                    if should_show_failure(report_late_failures, runtime) {
                        let code = out.status.code().unwrap_or(-1);
                        crate::platform::show_error_box(
                            &failure_message(&command, code, &stderr),
                            "Usage Monitor - Event Command Failed",
                        );
                    }
                }
                Err(err) => eprintln!("[event command] {command} failed to launch: {err}"),
            }
        });
    }
}

fn shell_command(command: &str) -> Command {
    if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", command]);
        c
    } else {
        let mut c = Command::new("sh");
        c.args(["-c", command]);
        c
    }
}

fn should_show_failure(report_late_failures: bool, runtime: f64) -> bool {
    report_late_failures || runtime <= STARTUP_FAILURE_WINDOW
}

fn failure_message(command: &str, returncode: i32, stderr: &str) -> String {
    let detail = stderr.trim();
    let detail = if detail.is_empty() {
        "(no error output on stderr)"
    } else {
        detail
    };
    format!("The event command exited with code {returncode}:\n\n{command}\n\n{detail}")
}

fn round_pct(pct: f64) -> String {
    format!("{}", pct.round() as i64)
}

pub fn reset_env(
    variant: &str,
    pct: f64,
    prev_pct: f64,
    data: &Map<String, Value>,
    entry: &Map<String, Value>,
) -> Vec<(String, String)> {
    let pct_5h = data
        .get("five_hour")
        .and_then(|v| v.get("utilization"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let pct_7d = data
        .get("seven_day")
        .and_then(|v| v.get("utilization"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let resets_at = entry.get("resets_at").and_then(Value::as_str).unwrap_or("");
    vec![
        ("USAGE_MONITOR_EVENT".into(), "reset".into()),
        ("USAGE_MONITOR_VARIANT".into(), variant.into()),
        ("USAGE_MONITOR_UTILIZATION".into(), round_pct(pct)),
        ("USAGE_MONITOR_PREV_UTILIZATION".into(), round_pct(prev_pct)),
        ("USAGE_MONITOR_UTILIZATION_FIVE_HOUR".into(), round_pct(pct_5h)),
        ("USAGE_MONITOR_UTILIZATION_SEVEN_DAY".into(), round_pct(pct_7d)),
        ("USAGE_MONITOR_RESETS_AT".into(), resets_at.into()),
        ("USAGE_MONITOR_TITLE".into(), t("notify_reset_title")),
        ("USAGE_MONITOR_MESSAGE".into(), t("notify_reset")),
    ]
}

pub fn threshold_env(
    variant: &str,
    pct: Option<f64>,
    threshold: f64,
    entry: &Map<String, Value>,
    title: &str,
    message: &str,
    extra_used: &str,
    extra_limit: &str,
) -> Vec<(String, String)> {
    let mut env = vec![
        ("USAGE_MONITOR_EVENT".into(), "threshold".into()),
        ("USAGE_MONITOR_VARIANT".into(), variant.into()),
    ];
    if let Some(pct) = pct {
        env.push(("USAGE_MONITOR_UTILIZATION".into(), round_pct(pct)));
    }
    env.push(("USAGE_MONITOR_THRESHOLD".into(), round_pct(threshold)));
    env.push((
        "USAGE_MONITOR_RESETS_AT".into(),
        entry.get("resets_at").and_then(Value::as_str).unwrap_or("").into(),
    ));
    env.push(("USAGE_MONITOR_TITLE".into(), title.into()));
    env.push(("USAGE_MONITOR_MESSAGE".into(), message.into()));
    if !extra_used.is_empty() {
        env.push(("USAGE_MONITOR_EXTRA_USED".into(), extra_used.into()));
    }
    if !extra_limit.is_empty() {
        env.push(("USAGE_MONITOR_EXTRA_LIMIT".into(), extra_limit.into()));
    }
    env
}

fn future_iso(hours: i64, days: i64) -> String {
    (Utc::now() + Duration::hours(hours) + Duration::days(days)).to_rfc3339()
}

pub fn run_test_command(kind: &str, state: &AppState) {
    let settings = state.settings.lock().clone();
    match kind {
        "test_reset_5h" => run_event_command_captured(&settings.on_reset_command, &test_reset_5h_env(), true),
        "test_reset_7d" => run_event_command_captured(&settings.on_reset_command, &test_reset_7d_env(), true),
        "test_threshold_5h" => {
            run_event_command_captured(&settings.on_threshold_command, &test_threshold_5h_env(), true)
        }
        "test_threshold_7d" => {
            run_event_command_captured(&settings.on_threshold_command, &test_threshold_7d_env(), true)
        }
        "test_startup" => run_event_command_captured(&settings.on_startup_command, &test_startup_env(), true),
        "test_quick_action" => {
            run_event_command_captured(&settings.quick_action_command, &test_quick_action_env(), true)
        }
        _ => {}
    }
}

fn test_reset_5h_env() -> Vec<(String, String)> {
    vec![
        ("USAGE_MONITOR_EVENT".into(), "reset".into()),
        ("USAGE_MONITOR_VARIANT".into(), "five_hour".into()),
        ("USAGE_MONITOR_UTILIZATION".into(), "0".into()),
        ("USAGE_MONITOR_PREV_UTILIZATION".into(), "95".into()),
        ("USAGE_MONITOR_UTILIZATION_FIVE_HOUR".into(), "0".into()),
        ("USAGE_MONITOR_UTILIZATION_SEVEN_DAY".into(), "45".into()),
        ("USAGE_MONITOR_RESETS_AT".into(), future_iso(5, 0)),
        ("USAGE_MONITOR_TITLE".into(), t("notify_reset_title")),
        ("USAGE_MONITOR_MESSAGE".into(), t("notify_reset")),
    ]
}

fn test_reset_7d_env() -> Vec<(String, String)> {
    vec![
        ("USAGE_MONITOR_EVENT".into(), "reset".into()),
        ("USAGE_MONITOR_VARIANT".into(), "seven_day".into()),
        ("USAGE_MONITOR_UTILIZATION".into(), "0".into()),
        ("USAGE_MONITOR_PREV_UTILIZATION".into(), "99".into()),
        ("USAGE_MONITOR_UTILIZATION_FIVE_HOUR".into(), "12".into()),
        ("USAGE_MONITOR_UTILIZATION_SEVEN_DAY".into(), "0".into()),
        ("USAGE_MONITOR_RESETS_AT".into(), future_iso(0, 7)),
        ("USAGE_MONITOR_TITLE".into(), t("notify_reset_title")),
        ("USAGE_MONITOR_MESSAGE".into(), t("notify_reset")),
    ]
}

fn test_threshold_5h_env() -> Vec<(String, String)> {
    let label = crate::formatting::popup_label("five_hour");
    vec![
        ("USAGE_MONITOR_EVENT".into(), "threshold".into()),
        ("USAGE_MONITOR_VARIANT".into(), "five_hour".into()),
        ("USAGE_MONITOR_UTILIZATION".into(), "82".into()),
        ("USAGE_MONITOR_THRESHOLD".into(), "80".into()),
        ("USAGE_MONITOR_RESETS_AT".into(), future_iso(3, 0)),
        ("USAGE_MONITOR_TITLE".into(), t("notify_threshold_title")),
        (
            "USAGE_MONITOR_MESSAGE".into(),
            t_fmt("notify_threshold_generic", &[("label", &label), ("pct", "82")]),
        ),
    ]
}

fn test_threshold_7d_env() -> Vec<(String, String)> {
    let label = crate::formatting::popup_label("seven_day");
    vec![
        ("USAGE_MONITOR_EVENT".into(), "threshold".into()),
        ("USAGE_MONITOR_VARIANT".into(), "seven_day".into()),
        ("USAGE_MONITOR_UTILIZATION".into(), "81".into()),
        ("USAGE_MONITOR_THRESHOLD".into(), "80".into()),
        ("USAGE_MONITOR_RESETS_AT".into(), future_iso(0, 4)),
        ("USAGE_MONITOR_TITLE".into(), t("notify_threshold_title")),
        (
            "USAGE_MONITOR_MESSAGE".into(),
            t_fmt("notify_threshold_generic", &[("label", &label), ("pct", "81")]),
        ),
    ]
}

fn test_startup_env() -> Vec<(String, String)> {
    vec![
        ("USAGE_MONITOR_EVENT".into(), "startup".into()),
        ("USAGE_MONITOR_UTILIZATION_FIVE_HOUR".into(), "0".into()),
        ("USAGE_MONITOR_RESETS_AT_FIVE_HOUR".into(), "".into()),
        ("USAGE_MONITOR_UTILIZATION_SEVEN_DAY".into(), "45".into()),
        ("USAGE_MONITOR_RESETS_AT_SEVEN_DAY".into(), future_iso(0, 3)),
    ]
}

fn test_quick_action_env() -> Vec<(String, String)> {
    vec![
        ("USAGE_MONITOR_EVENT".into(), "quick_action".into()),
        ("USAGE_MONITOR_UTILIZATION_FIVE_HOUR".into(), "30".into()),
        ("USAGE_MONITOR_RESETS_AT_FIVE_HOUR".into(), future_iso(3, 0)),
        ("USAGE_MONITOR_UTILIZATION_SEVEN_DAY".into(), "55".into()),
        ("USAGE_MONITOR_RESETS_AT_SEVEN_DAY".into(), future_iso(0, 4)),
    ]
}

pub fn quota_env(data: &Map<String, Value>) -> Vec<(String, String)> {
    let mut env = Vec::new();
    for (key, value) in data {
        if key == "extra_usage" {
            continue;
        }
        let Some(obj) = value.as_object() else { continue };
        if !obj.contains_key("utilization") {
            continue;
        }
        let util = obj.get("utilization").and_then(Value::as_f64).unwrap_or(0.0);
        env.push((
            format!("USAGE_MONITOR_UTILIZATION_{}", key.to_uppercase()),
            format!("{}", util.round() as i64),
        ));
        let reset = obj.get("resets_at").and_then(Value::as_str).unwrap_or("");
        env.push((format!("USAGE_MONITOR_RESETS_AT_{}", key.to_uppercase()), reset.into()));
    }
    if let Some(extra) = data.get("extra_usage").and_then(Value::as_object) {
        if extra.get("is_enabled").and_then(Value::as_bool) == Some(true) {
            let used = extra.get("used_credits").and_then(Value::as_f64).unwrap_or(0.0);
            let currency = extra.get("currency").and_then(Value::as_str);
            let places = extra.get("decimal_places").and_then(Value::as_i64);
            env.push(("USAGE_MONITOR_EXTRA_USED".into(), format_credits(used, currency, places)));
            let limit = extra.get("monthly_limit").and_then(Value::as_f64).unwrap_or(0.0);
            if limit > 0.0 {
                env.push(("USAGE_MONITOR_EXTRA_LIMIT".into(), format_credits(limit, currency, places)));
            }
        }
    }
    env
}

pub fn run_quick_action(state: &AppState) {
    let cmds = state.settings.lock().quick_action_command.clone();
    if cmds.is_empty() {
        return;
    }
    let data = state.last_response.lock().clone();
    let mut env = quota_env(&data);
    env.push(("USAGE_MONITOR_EVENT".into(), "quick_action".into()));
    run_event_command_captured(&cmds, &env, false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    fn env_map(data: Value) -> HashMap<String, String> {
        quota_env(data.as_object().unwrap()).into_iter().collect()
    }

    #[test]
    fn quota_env_snapshot_matches_python() {
        let env = env_map(json!({
            "five_hour": {"utilization": 30.4, "resets_at": "2026-01-01T00:00:00Z"},
            "seven_day": {"utilization": 55.6},
            "error": "nope",
            "extra_usage": {
                "is_enabled": true,
                "used_credits": 820.0,
                "monthly_limit": 1000.0,
                "currency": "USD",
                "decimal_places": 2
            }
        }));
        assert_eq!(env.get("USAGE_MONITOR_UTILIZATION_FIVE_HOUR").unwrap(), "30");
        assert_eq!(env.get("USAGE_MONITOR_RESETS_AT_FIVE_HOUR").unwrap(), "2026-01-01T00:00:00Z");
        assert_eq!(env.get("USAGE_MONITOR_UTILIZATION_SEVEN_DAY").unwrap(), "56");
        assert_eq!(env.get("USAGE_MONITOR_RESETS_AT_SEVEN_DAY").unwrap(), "");
        assert_eq!(env.get("USAGE_MONITOR_EXTRA_USED").unwrap(), "8.20");
        assert_eq!(env.get("USAGE_MONITOR_EXTRA_LIMIT").unwrap(), "10.00");
        assert!(!env.contains_key("USAGE_MONITOR_UTILIZATION_ERROR"));
        assert!(!env.contains_key("USAGE_MONITOR_UTILIZATION_EXTRA_USAGE"));
    }

    #[test]
    fn quota_env_enabled_extra_without_limit() {
        let env = env_map(json!({
            "five_hour": {"utilization": 1.0, "resets_at": ""},
            "extra_usage": {"is_enabled": true, "used_credits": 50.0, "monthly_limit": 0.0, "decimal_places": 2}
        }));
        assert_eq!(env.get("USAGE_MONITOR_EXTRA_USED").unwrap(), "0.50");
        assert!(!env.contains_key("USAGE_MONITOR_EXTRA_LIMIT"));
    }

    #[test]
    fn quota_env_skips_disabled_extra() {
        let env = env_map(json!({
            "five_hour": {"utilization": 1.0, "resets_at": ""},
            "extra_usage": {"is_enabled": false, "used_credits": 1.0, "monthly_limit": 10.0}
        }));
        assert!(!env.contains_key("USAGE_MONITOR_EXTRA_USED"));
        assert!(!env.contains_key("USAGE_MONITOR_EXTRA_LIMIT"));
    }

    fn env_lookup<'a>(env: &'a [(String, String)], key: &str) -> Option<&'a str> {
        env.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    #[test]
    fn reset_env_has_python_keys() {
        let data = json!({
            "five_hour": {"utilization": 12.4, "resets_at": "soon"},
            "seven_day": {"utilization": 45.6, "resets_at": "later"}
        });
        let data = data.as_object().unwrap();
        let entry = data.get("five_hour").unwrap().as_object().unwrap();
        let env = reset_env("five_hour", 12.4, 95.2, data, entry);
        assert_eq!(env_lookup(&env, "USAGE_MONITOR_EVENT"), Some("reset"));
        assert_eq!(env_lookup(&env, "USAGE_MONITOR_VARIANT"), Some("five_hour"));
        assert_eq!(env_lookup(&env, "USAGE_MONITOR_UTILIZATION"), Some("12"));
        assert_eq!(env_lookup(&env, "USAGE_MONITOR_PREV_UTILIZATION"), Some("95"));
        assert_eq!(env_lookup(&env, "USAGE_MONITOR_UTILIZATION_FIVE_HOUR"), Some("12"));
        assert_eq!(env_lookup(&env, "USAGE_MONITOR_UTILIZATION_SEVEN_DAY"), Some("46"));
        assert_eq!(env_lookup(&env, "USAGE_MONITOR_RESETS_AT"), Some("soon"));
        assert!(env_lookup(&env, "USAGE_MONITOR_TITLE").is_some());
        assert!(env_lookup(&env, "USAGE_MONITOR_MESSAGE").is_some());
    }

    #[test]
    fn threshold_env_omits_pct_for_spent() {
        let entry = json!({"resets_at": ""}).as_object().cloned().unwrap();
        let env = threshold_env(
            "extra_usage_spent",
            None,
            50.0,
            &entry,
            "title",
            "msg",
            "$50.00",
            "",
        );
        assert_eq!(env_lookup(&env, "USAGE_MONITOR_EVENT"), Some("threshold"));
        assert_eq!(env_lookup(&env, "USAGE_MONITOR_VARIANT"), Some("extra_usage_spent"));
        assert!(env_lookup(&env, "USAGE_MONITOR_UTILIZATION").is_none());
        assert_eq!(env_lookup(&env, "USAGE_MONITOR_THRESHOLD"), Some("50"));
        assert_eq!(env_lookup(&env, "USAGE_MONITOR_EXTRA_USED"), Some("$50.00"));
        assert!(env_lookup(&env, "USAGE_MONITOR_EXTRA_LIMIT").is_none());
        assert_eq!(env_lookup(&env, "USAGE_MONITOR_TITLE"), Some("title"));
        assert_eq!(env_lookup(&env, "USAGE_MONITOR_MESSAGE"), Some("msg"));
    }

    #[test]
    fn test_fixtures_match_python_keys() {
        let reset = test_reset_5h_env();
        assert_eq!(env_lookup(&reset, "USAGE_MONITOR_VARIANT"), Some("five_hour"));
        assert_eq!(env_lookup(&reset, "USAGE_MONITOR_UTILIZATION"), Some("0"));
        assert_eq!(env_lookup(&reset, "USAGE_MONITOR_PREV_UTILIZATION"), Some("95"));
        let th = test_threshold_7d_env();
        assert_eq!(env_lookup(&th, "USAGE_MONITOR_VARIANT"), Some("seven_day"));
        assert_eq!(env_lookup(&th, "USAGE_MONITOR_THRESHOLD"), Some("80"));
        let start = test_startup_env();
        assert_eq!(env_lookup(&start, "USAGE_MONITOR_EVENT"), Some("startup"));
        assert_eq!(env_lookup(&start, "USAGE_MONITOR_UTILIZATION_FIVE_HOUR"), Some("0"));
        let qa = test_quick_action_env();
        assert_eq!(env_lookup(&qa, "USAGE_MONITOR_EVENT"), Some("quick_action"));
        assert_eq!(env_lookup(&qa, "USAGE_MONITOR_UTILIZATION_SEVEN_DAY"), Some("55"));
    }

    #[test]
    fn late_failures_hidden_unless_flag() {
        assert!(should_show_failure(true, 60.0));
        assert!(!should_show_failure(false, 60.0));
        assert!(should_show_failure(false, 5.0));
        assert!(should_show_failure(false, 0.2));
    }

    #[test]
    fn failure_message_uses_python_copy() {
        let msg = failure_message("boom", 1, "  err  ");
        assert!(msg.contains("The event command exited with code 1:"));
        assert!(msg.contains("boom"));
        assert!(msg.contains("err"));
        let empty = failure_message("x", 2, "   ");
        assert!(empty.contains("(no error output on stderr)"));
    }
}

