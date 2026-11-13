mod alerts;
mod cache;
mod claude_cli;
mod claude_login;
mod command;
mod dismiss;
mod error;
mod formatting;
mod history;
mod http;
mod i18n;
mod instance;
mod platform;
mod poll;
mod popup;
mod provider_updater;
mod settings;
mod sources;
mod state;
mod tray;
mod tray_icon;
mod types;
mod updater;

use std::sync::Arc;
use std::time::Duration;

use tauri::Manager;
use tauri_plugin_autostart::MacosLauncher;

use crate::error::Result;
use crate::settings::Settings;
use crate::state::AppState;
use crate::types::{CustomPayload, SettingsView, Source};

const AUTOSTART_DELAY_SECONDS: u64 = 30;

#[tauri::command]
fn close_popup(app: tauri::AppHandle, state: tauri::State<Arc<AppState>>) -> Result<()> {
    popup::hide(&app, &state)
}

#[tauri::command]
fn open_dashboard() -> Result<()> {
    popup::open_dashboard()
}

/// Minimum gap between manual refreshes; extra clicks inside it are ignored.
const MANUAL_REFRESH_MIN_GAP: Duration = Duration::from_secs(2);
static LAST_MANUAL_REFRESH: parking_lot::Mutex<Option<std::time::Instant>> = parking_lot::Mutex::new(None);

/// Records a manual refresh at `now` and returns true unless one happened less than `gap` ago.
fn manual_refresh_allowed(last: &mut Option<std::time::Instant>, now: std::time::Instant, gap: Duration) -> bool {
    if let Some(prev) = *last {
        if now.saturating_duration_since(prev) < gap {
            return false;
        }
    }
    *last = Some(now);
    true
}

#[tauri::command]
fn refresh_now(app: tauri::AppHandle, state: tauri::State<Arc<AppState>>) -> Result<()> {
    if manual_refresh_allowed(&mut LAST_MANUAL_REFRESH.lock(), std::time::Instant::now(), MANUAL_REFRESH_MIN_GAP) {
        poll::force_update(app, state.inner().clone());
    }
    Ok(())
}

/// Run a settings mutation (which rebuilds the settings view and may therefore run slow
/// source discovery) off the UI thread.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| crate::error::Error::from(e.to_string()))?
}

#[tauri::command]
fn open_link(url: String) -> Result<()> {
    let Some(parsed) = validate_link(&url, &allowed_link_hosts()) else {
        return Ok(());
    };
    let url = parsed.as_str();
    #[cfg(target_os = "windows")]
    shell_open(url);
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(url).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    }
    Ok(())
}

/// Hosts the app itself links to: provider key/dashboard pages from the keyed catalog plus
/// the fixed dashboards of the Claude web and Antigravity sources.
fn allowed_link_hosts() -> Vec<String> {
    let mut hosts: Vec<String> = vec!["claude.ai".into(), "antigravity.google".into()];
    for def in sources::keyed::CATALOG {
        for u in [def.key_url, def.dashboard_url] {
            if let Some(h) = url::Url::parse(u).ok().and_then(|u| u.host_str().map(str::to_ascii_lowercase)) {
                hosts.push(h);
            }
        }
    }
    hosts
}

/// Validate a link requested by the UI: https only, no credentials, default port, and an
/// exact host from the allowlist.
fn validate_link(raw: &str, allowed: &[String]) -> Option<url::Url> {
    let parsed = url::Url::parse(raw.trim()).ok()?;
    if parsed.scheme() != "https" || !parsed.username().is_empty() || parsed.password().is_some() {
        return None;
    }
    if parsed.port().is_some() {
        return None;
    }
    let host = parsed.host_str()?.trim_end_matches('.').to_ascii_lowercase();
    allowed.iter().any(|a| *a == host).then_some(parsed)
}

#[cfg(target_os = "windows")]
fn shell_open(url: &str) {
    use std::os::windows::ffi::OsStrExt;
    let wide = |s: &str| std::ffi::OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect::<Vec<u16>>();
    let (verb, target) = (wide("open"), wide(url));
    // SAFETY: both buffers are NUL-terminated and outlive the call; the URL is passed as a
    // single argument and is never interpreted by a shell.
    unsafe {
        windows_sys::Win32::UI::Shell::ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            target.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        );
    }
}

#[tauri::command]
fn set_pinned(state: tauri::State<Arc<AppState>>, pinned: bool) -> Result<bool> {
    *state.popup_pinned.lock() = pinned;
    if !pinned {
        *state.popup_moved.lock() = false;
    }
    crate::dismiss::sync(&state);
    Ok(pinned)
}

#[tauri::command]
fn begin_drag(app: tauri::AppHandle, state: tauri::State<Arc<AppState>>) -> Result<bool> {
    popup::begin_drag(&app, &state)
}

#[tauri::command]
fn drag(app: tauri::AppHandle, state: tauri::State<Arc<AppState>>) -> Result<bool> {
    popup::drag(&app, &state)
}

#[tauri::command]
fn end_drag(app: tauri::AppHandle, state: tauri::State<Arc<AppState>>) -> Result<()> {
    popup::end_drag(&app, &state)
}

#[tauri::command]
fn report_height(app: tauri::AppHandle, state: tauri::State<Arc<AppState>>, height: i32) -> Result<()> {
    popup::apply_height(&app, &state, height)
}

#[tauri::command]
async fn load_settings(state: tauri::State<'_, Arc<AppState>>) -> Result<SettingsView> {
    // Source discovery can touch disk, the credential manager, a child process and the
    // network. Snapshot the settings, release the lock and run it off the UI thread.
    let snapshot = state.settings.lock().clone();
    tauri::async_runtime::spawn_blocking(move || snapshot.view())
        .await
        .map_err(|e| crate::error::Error::from(e.to_string()))
}

#[tauri::command]
fn get_provider_auto_update(state: tauri::State<'_, Arc<AppState>>) -> bool {
    state.settings.lock().provider_auto_update
}

#[tauri::command]
async fn set_provider_auto_update(state: tauri::State<'_, Arc<AppState>>, enabled: bool) -> Result<crate::types::ProviderUpdatePreference> {
    let state = state.inner().clone();
    blocking(move || {
        let mut settings = state.settings.lock();
        let error = settings.set_provider_auto_update(enabled).err().map(|e| e.to_string());
        Ok(crate::types::ProviderUpdatePreference { enabled: settings.provider_auto_update, error })
    }).await
}

#[tauri::command]
async fn save_ninerouter(state: tauri::State<'_, Arc<AppState>>, url: String) -> Result<SettingsView> {
    let state = state.inner().clone();
    blocking(move || settings::save_ninerouter(&state, url)).await
}

#[tauri::command]
async fn add_custom(app: tauri::AppHandle, state: tauri::State<'_, Arc<AppState>>, payload: CustomPayload) -> Result<SettingsView> {
    let state = state.inner().clone();
    blocking(move || {
        let view = settings::add_custom(&state, payload)?;
        crate::tray::refresh(&app, &state);
        Ok(view)
    })
    .await
}

#[tauri::command]
fn test_custom(payload: CustomPayload) -> Result<types::CustomTestResult> {
    sources::custom::probe(&payload.url, &payload.header, &payload.token)
}

#[tauri::command]
async fn remove_custom(app: tauri::AppHandle, state: tauri::State<'_, Arc<AppState>>, id: String) -> Result<SettingsView> {
    let state = state.inner().clone();
    blocking(move || {
        let view = settings::remove_custom(&state, &id)?;
        if sources::ensure_visible_selection(&state) {
            poll::force_update(app.clone(), state.clone());
        }
        crate::tray::refresh(&app, &state);
        Ok(view)
    })
    .await
}

#[tauri::command]
async fn add_keyed(
    app: tauri::AppHandle,
    state: tauri::State<'_, Arc<AppState>>,
    provider: String,
    key: String,
    name: String,
) -> Result<SettingsView> {
    // Test first (blocking HTTP, off the main thread); only a working key is saved.
    let (p, k) = (provider.clone(), key.clone());
    tauri::async_runtime::spawn_blocking(move || sources::keyed::test(&p, &k))
        .await
        .map_err(|e| crate::error::Error::from(e.to_string()))??;
    let state = state.inner().clone();
    blocking(move || {
        let view = settings::add_keyed(&state, &provider, &key, &name)?;
        crate::tray::refresh(&app, &state);
        Ok(view)
    })
    .await
}

#[tauri::command]
async fn remove_keyed(app: tauri::AppHandle, state: tauri::State<'_, Arc<AppState>>, id: String) -> Result<SettingsView> {
    let state = state.inner().clone();
    blocking(move || {
        let view = settings::remove_keyed(&state, &id)?;
        if sources::ensure_visible_selection(&state) {
            poll::force_update(app.clone(), state.clone());
        }
        crate::tray::refresh(&app, &state);
        Ok(view)
    })
    .await
}

#[tauri::command]
async fn set_source_visible(app: tauri::AppHandle, state: tauri::State<'_, Arc<AppState>>, id: String, visible: bool) -> Result<SettingsView> {
    let state = state.inner().clone();
    blocking(move || {
        let view = settings::set_source_visible(&state, id, visible)?;
        if sources::ensure_visible_selection(&state) {
            poll::force_update(app.clone(), state.clone());
        }
        crate::tray::refresh(&app, &state);
        Ok(view)
    })
    .await
}

#[tauri::command]
async fn set_show_remaining(app: tauri::AppHandle, state: tauri::State<'_, Arc<AppState>>, remaining: bool) -> Result<SettingsView> {
    let state = state.inner().clone();
    blocking(move || {
        let view = settings::set_show_remaining(&state, remaining)?;
        crate::tray::refresh(&app, &state);
        crate::alerts::emit_update(&app, &state);
        Ok(view)
    })
    .await
}

/// Language codes the UI can offer, as `(code, display name)` pairs.
#[tauri::command]
fn list_languages() -> Vec<(String, String)> {
    crate::i18n::available_languages()
}

/// The popup-facing translation strings for one language code, so the UI can
/// re-render immediately after a language change without a restart.
#[tauri::command]
fn translations_for(code: String) -> serde_json::Value {
    popup::translations_payload(&code)
}

#[tauri::command]
async fn set_language(state: tauri::State<'_, Arc<AppState>>, code: String) -> Result<SettingsView> {
    let state = state.inner().clone();
    blocking(move || settings::set_language(&state, code)).await
}

#[tauri::command]
fn rename_bar(app: tauri::AppHandle, state: tauri::State<Arc<AppState>>, key: String, name: String) -> Result<()> {
    let mut settings = state.settings.lock();
    settings.set_bar_label(&key, &name)?;
    drop(settings);
    crate::tray::refresh(&app, &state);
    crate::alerts::emit_update(&app, &state);
    Ok(())
}

#[tauri::command]
fn cycle_group_account(app: tauri::AppHandle, state: tauri::State<Arc<AppState>>, direction: i32) -> Result<()> {
    sources::cycle_group_account(&state, direction);
    poll::force_update(app, state.inner().clone());
    Ok(())
}

#[tauri::command]
fn get_popup_init(state: tauri::State<Arc<AppState>>) -> Result<types::PopupInit> {
    popup::init_payload(&state)
}

#[tauri::command]
async fn list_sources(state: tauri::State<'_, Arc<AppState>>) -> Result<Vec<Source>> {
    // Snapshot, release the lock, discover off the UI thread.
    let snapshot = state.settings.lock().clone();
    blocking(move || Ok(sources::list_sources(&snapshot))).await
}

#[tauri::command]
fn select_source(app: tauri::AppHandle, state: tauri::State<Arc<AppState>>, id: String) -> Result<()> {
    sources::select_source(&state, id);
    poll::force_update(app, state.inner().clone());
    Ok(())
}

#[tauri::command]
async fn claude_web_sign_in(app: tauri::AppHandle, state: tauri::State<'_, Arc<AppState>>) -> Result<()> {
    sources::claude_web::open_login(&app, state.inner().clone()).map_err(crate::error::Error::from)
}

/// True when the Claude CLI is installed, so the UI can offer the browser sign-in.
#[tauri::command]
fn claude_cli_available() -> bool {
    crate::claude_cli::claude_path().is_file()
}

/// Start `claude auth login` in the background; the result arrives as the
/// `claude-login://done` event. Returns at once.
#[tauri::command]
fn claude_cli_sign_in(app: tauri::AppHandle, state: tauri::State<'_, Arc<AppState>>) -> Result<()> {
    claude_login::start(&app, state.inner().clone()).map_err(crate::error::Error::from)
}

/// Abort a running CLI sign-in (kills the CLI process).
#[tauri::command]
fn claude_cli_cancel() {
    claude_login::cancel();
}

#[tauri::command]
async fn claude_cli_sign_out(app: tauri::AppHandle, state: tauri::State<'_, Arc<AppState>>) -> Result<SettingsView> {
    let state = state.inner().clone();
    blocking(move || {
        claude_login::sign_out().map_err(crate::error::Error::from)?;
        if sources::ensure_visible_selection(&state) {
            poll::force_update(app.clone(), state.clone());
        }
        crate::tray::refresh(&app, &state);
        let snapshot = state.settings.lock().clone();
        Ok(snapshot.view())
    })
    .await
}

#[tauri::command]
async fn claude_web_sign_out(app: tauri::AppHandle, state: tauri::State<'_, Arc<AppState>>) -> Result<SettingsView> {
    let state = state.inner().clone();
    blocking(move || {
        sources::claude_web::sign_out(&app);
        sources::invalidate_caches();
        if sources::ensure_visible_selection(&state) {
            poll::force_update(app.clone(), state.clone());
        }
        crate::tray::refresh(&app, &state);
        let snapshot = state.settings.lock().clone();
        Ok(snapshot.view())
    })
    .await
}

#[tauri::command]
fn set_autostart(app: tauri::AppHandle, enable: bool) -> Result<()> {
    platform::set_autostart(&app, enable)
}

#[tauri::command]
fn is_autostart(app: tauri::AppHandle) -> Result<bool> {
    platform::is_autostart(&app)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let mut autostart = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--verbose" {
            std::env::set_var("USAGE_MONITOR_VERBOSE", "1");
        } else if let Some(value) = arg.strip_prefix("--config-dir=") {
            std::env::set_var("CLAUDE_CONFIG_DIR", value);
        } else if arg == "--config-dir" {
            if let Some(value) = args.next() {
                std::env::set_var("CLAUDE_CONFIG_DIR", value);
            }
        } else if let Some(value) = arg.strip_prefix("--ninerouter-url=") {
            std::env::set_var("NINEROUTER_URL", value.trim_end_matches('/'));
        } else if arg == "--ninerouter-url" {
            if let Some(value) = args.next() {
                std::env::set_var("NINEROUTER_URL", value.trim_end_matches('/'));
            }
        } else if let Some(value) = arg.strip_prefix("--connection=") {
            std::env::set_var("USAGE_MONITOR_CONNECTION", value);
        } else if arg == "--connection" {
            if let Some(value) = args.next() {
                std::env::set_var("USAGE_MONITOR_CONNECTION", value);
            }
        } else if arg == "--autostart" {
            autostart = true;
        }
    }

    platform::ensure_gui_path();

    let settings = Settings::load();
    let state = AppState::new(settings);

    let exit_state = state.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(state) = app.try_state::<Arc<AppState>>() {
                let _ = popup::show(app, state.inner());
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--autostart".into()]),
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![
            close_popup,
            open_dashboard,
            refresh_now,
            open_link,
            set_pinned,
            begin_drag,
            drag,
            end_drag,
            report_height,
            load_settings,
            set_provider_auto_update,
            get_provider_auto_update,
            save_ninerouter,
            add_custom,
            test_custom,
            remove_custom,
            add_keyed,
            remove_keyed,
            set_source_visible,
            set_show_remaining,
            list_languages,
            translations_for,
            set_language,
            rename_bar,
            cycle_group_account,
            get_popup_init,
            list_sources,
            select_source,
            claude_web_sign_in,
            claude_web_sign_out,
            claude_cli_available,
            claude_cli_sign_in,
            claude_cli_sign_out,
            claude_cli_cancel,
            set_autostart,
            is_autostart,
        ])
        .setup(move |app| {
            if autostart {
                std::thread::sleep(Duration::from_secs(AUTOSTART_DELAY_SECONDS));
            }

            #[cfg(target_os = "macos")]
            {
                crate::platform::clear_macos_quarantine();
                let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
                use tauri_plugin_notification::NotificationExt;
                let _ = app.notification().request_permission();
                if let Some(w) = app.get_webview_window("popup") {
                    let _ = w.set_shadow(true);
                    let _ = w.set_visible_on_all_workspaces(true);
                    let handle = app.handle().clone();
                    let state_w = state.clone();
                    w.on_window_event(move |event| {
                        if let tauri::WindowEvent::Focused(false) = event {
                            if *state_w.popup_shown.lock() && !*state_w.popup_pinned.lock() {
                                let _ = popup::hide(&handle, &state_w);
                            }
                        }
                    });
                }
            }
            sources::claude_web::init(app.handle());
            tray::setup(app.handle(), state.clone())?;
            poll::spawn(app.handle().clone(), state.clone());
            updater::spawn(app.handle().clone(), state.clone());
            provider_updater::spawn(app.handle().clone(), state.clone());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Usage Monitor")
        .run(move |_app, event| {
            if let tauri::RunEvent::Exit = event {
                crate::history::save_throttled(&exit_state.usage_history.lock(), crate::history::now_secs(), true);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn link_validator_accepts_only_known_https_hosts() {
        let allowed = allowed_link_hosts();
        for ok in [
            "https://claude.ai/settings/usage",
            "https://openrouter.ai/settings/keys",
            "https://antigravity.google",
            "https://github.com/settings/tokens",
        ] {
            assert!(validate_link(ok, &allowed).is_some(), "{ok}");
        }
        for bad in [
            "http://claude.ai/",
            "https://claude.ai.evil.com/",
            "https://evilclaude.ai/",
            "https://claude.ai@evil.com/",
            "https://user:pw@claude.ai/",
            "https://claude.ai:8443/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "https://example.com/",
            "claude.ai",
            "",
        ] {
            assert!(validate_link(bad, &allowed).is_none(), "{bad}");
        }
    }

    #[test]
    fn manual_refresh_is_rate_limited() {
        let mut last = None;
        let t0 = Instant::now();
        assert!(manual_refresh_allowed(&mut last, t0, MANUAL_REFRESH_MIN_GAP));
        assert!(!manual_refresh_allowed(&mut last, t0 + Duration::from_millis(500), MANUAL_REFRESH_MIN_GAP));
        assert!(manual_refresh_allowed(&mut last, t0 + Duration::from_secs(3), MANUAL_REFRESH_MIN_GAP));
    }
}
