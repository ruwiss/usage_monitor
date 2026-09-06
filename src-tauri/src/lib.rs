mod alerts;
mod cache;
mod claude_cli;
mod command;
mod dismiss;
mod error;
mod formatting;
mod http;
mod i18n;
mod instance;
mod platform;
mod poll;
mod popup;
mod settings;
mod sources;
mod state;
mod tray;
mod tray_icon;
mod types;

use std::sync::Arc;

use tauri::Manager;
use tauri_plugin_autostart::MacosLauncher;

use crate::error::Result;
use crate::settings::Settings;
use crate::state::AppState;
use crate::types::{CustomPayload, SettingsView, Source};

#[tauri::command]
fn close_popup(app: tauri::AppHandle, state: tauri::State<Arc<AppState>>) -> Result<()> {
    popup::hide(&app, &state)
}

#[tauri::command]
fn open_dashboard() -> Result<()> {
    popup::open_dashboard()
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
fn load_settings(state: tauri::State<Arc<AppState>>) -> Result<SettingsView> {
    Ok(state.settings.lock().view())
}

#[tauri::command]
fn save_ninerouter(state: tauri::State<Arc<AppState>>, url: String) -> Result<SettingsView> {
    settings::save_ninerouter(&state, url)
}

#[tauri::command]
fn add_custom(state: tauri::State<Arc<AppState>>, payload: CustomPayload) -> Result<SettingsView> {
    settings::add_custom(&state, payload)
}

#[tauri::command]
fn test_custom(payload: CustomPayload) -> Result<types::CustomTestResult> {
    sources::custom::probe(&payload.url, &payload.header, &payload.token)
}

#[tauri::command]
fn remove_custom(state: tauri::State<Arc<AppState>>, id: String) -> Result<SettingsView> {
    settings::remove_custom(&state, &id)
}

#[tauri::command]
fn set_show_remaining(app: tauri::AppHandle, state: tauri::State<Arc<AppState>>, remaining: bool) -> Result<SettingsView> {
    let view = settings::set_show_remaining(&state, remaining)?;
    crate::tray::refresh(&app, &state);
    crate::alerts::emit_update(&app, &state);
    Ok(view)
}

#[tauri::command]
fn get_popup_init(state: tauri::State<Arc<AppState>>) -> Result<types::PopupInit> {
    popup::init_payload(&state)
}

#[tauri::command]
fn list_sources(state: tauri::State<Arc<AppState>>) -> Result<Vec<Source>> {
    Ok(sources::list_sources(&state.settings.lock()))
}

#[tauri::command]
fn select_source(app: tauri::AppHandle, state: tauri::State<Arc<AppState>>, id: String) -> Result<()> {
    sources::select_source(&state, id);
    poll::force_update(app, state.inner().clone());
    Ok(())
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
        }
    }

    platform::ensure_gui_path();

    let settings = Settings::load();
    let state = AppState::new(settings);

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(state) = app.try_state::<Arc<AppState>>() {
                let _ = popup::show(app, state.inner());
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_notification::init())
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![
            close_popup,
            open_dashboard,
            set_pinned,
            begin_drag,
            drag,
            end_drag,
            report_height,
            load_settings,
            save_ninerouter,
            add_custom,
            test_custom,
            remove_custom,
            set_show_remaining,
            get_popup_init,
            list_sources,
            select_source,
            set_autostart,
            is_autostart,
        ])
        .setup(move |app| {
            #[cfg(target_os = "macos")]
            {
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
            tray::setup(app.handle(), state.clone())?;
            poll::spawn(app.handle().clone(), state.clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Usage Monitor");
}
