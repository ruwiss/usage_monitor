use crate::i18n::{t, t_fmt};
use crate::settings::Settings;
use crate::state::AppState;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::UpdaterExt;

/// Static release manifest. This is a file download, not the GitHub REST API.
const ENDPOINT: &str = "https://github.com/ruwiss/usage_monitor/releases/latest/download/latest.json";
const CHECK_INTERVAL_SECS: i64 = 6 * 60 * 60;

pub fn spawn(app: tauri::AppHandle, state: Arc<AppState>) {
    tauri::async_runtime::spawn(async move {
        if let Err(err) = check_and_apply(app, state).await {
            eprintln!("updater: {err}");
        }
    });
}

async fn check_and_apply(app: tauri::AppHandle, state: Arc<AppState>) -> Result<(), String> {
    if !should_check(&state.settings.lock()) {
        return Ok(());
    }

    let updater = app
        .updater_builder()
        .endpoints(vec![ENDPOINT.parse().map_err(|err| format!("{err}"))?])
        .map_err(|err| err.to_string())?
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|err| err.to_string())?;

    let update = match updater.check().await {
        Ok(update) => {
            mark_checked(&state);
            update
        }
        Err(err) => {
            let msg = err.to_string();
            if !is_transient(&msg) {
                mark_checked(&state);
            }
            return Err(msg);
        }
    };

    let Some(update) = update else {
        return Ok(());
    };

    let version = update.version.clone();
    let _ = app
        .notification()
        .builder()
        .title(t("notify_app_update_title"))
        .body(t_fmt("notify_app_update", &[("version", &version)]))
        .show();

    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|err| err.to_string())?;
    app.restart();
}

pub fn should_check(settings: &Settings) -> bool {
    due(settings, chrono::Utc::now().timestamp(), cfg!(debug_assertions))
}

pub fn due(settings: &Settings, now: i64, debug: bool) -> bool {
    if debug || !settings.auto_update {
        return false;
    }
    settings.last_update_check <= 0 || now.saturating_sub(settings.last_update_check) >= CHECK_INTERVAL_SECS
}

fn mark_checked(state: &Arc<AppState>) {
    let now = chrono::Utc::now().timestamp();
    let _ = state.settings.lock().save_setting("last_update_check", json!(now));
}

fn is_transient(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("connection")
        || lower.contains("network")
        || lower.contains("dns")
        || lower.contains("reset")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(auto_update: bool, last: i64) -> Settings {
        let mut s = Settings::default();
        s.auto_update = auto_update;
        s.last_update_check = last;
        s
    }

    #[test]
    fn skips_debug_and_disabled() {
        let s = settings(true, 0);
        assert!(!due(&s, 1_000, true));
        let s = settings(false, 0);
        assert!(!due(&s, 1_000, false));
    }

    #[test]
    fn checks_when_never_checked() {
        let s = settings(true, 0);
        assert!(due(&s, 1_000, false));
    }

    #[test]
    fn respects_interval() {
        let s = settings(true, 1_000);
        assert!(!due(&s, 1_000 + CHECK_INTERVAL_SECS - 1, false));
        assert!(due(&s, 1_000 + CHECK_INTERVAL_SECS, false));
    }
}
