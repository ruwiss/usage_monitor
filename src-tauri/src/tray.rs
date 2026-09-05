use crate::command;
use crate::formatting::{elapsed_pct, field_period, format_tooltip};
use crate::i18n::t;
use crate::popup;
use crate::sources;
use crate::state::AppState;
use crate::tray_icon::create_icon_png;
use serde_json::Value;
use std::sync::Arc;
use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
#[cfg(windows)]
use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
use tauri::AppHandle;

#[cfg(any(windows, test))]
#[derive(Default)]
struct ClickGate {
    token: u64,
    swallow: bool,
}

#[cfg(any(windows, test))]
impl ClickGate {
    /// Left-button up. Some(token) schedules single-click; None swallows.
    fn on_left_up(&mut self) -> Option<u64> {
        if self.swallow {
            self.swallow = false;
            None
        } else {
            self.token += 1;
            Some(self.token)
        }
    }

    fn on_double_click(&mut self) {
        self.swallow = true;
        self.token += 1;
    }

    fn is_current(&self, token: u64) -> bool {
        self.token == token
    }
}

pub fn setup(app: &AppHandle, state: Arc<AppState>) -> tauri::Result<()> {
    *state.light_taskbar.lock() = crate::platform::light_taskbar();
    let menu = build_menu(app, &state)?;
    let png = icon_png(&state);
    let image = Image::from_bytes(&png).unwrap_or_else(|_| Image::new(&[], 0, 0));
    let state_menu = state.clone();
    TrayIconBuilder::with_id("main")
        .icon(image)
        .tooltip(&t("loading"))
        .menu(&menu)
        .show_menu_on_left_click(cfg!(not(windows)))
        .on_tray_icon_event({
            let state_click = state.clone();
            #[cfg(windows)]
            let defer_click = !state.settings.lock().quick_action_command.is_empty();
            #[cfg(windows)]
            let gate = Arc::new(parking_lot::Mutex::new(ClickGate::default()));
            move |tray, event| {
                #[cfg(not(windows))]
                {
                    let _ = (tray, event, &state_click);
                }
                #[cfg(windows)]
                {
                    handle_windows_click(tray, event, &state_click, defer_click, &gate);
                }
            }
        })
        .on_menu_event(move |app, event| {
            handle_menu(app, &state_menu, event.id().as_ref());
        })
        .build(app)?;
    Ok(())
}

#[cfg(windows)]
fn double_click_interval() -> std::time::Duration {
    std::time::Duration::from_millis(unsafe {
        windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() as u64
    })
}

#[cfg(windows)]
fn handle_windows_click(
    tray: &tauri::tray::TrayIcon,
    event: TrayIconEvent,
    state: &Arc<AppState>,
    defer: bool,
    gate: &Arc<parking_lot::Mutex<ClickGate>>,
) {
    match event {
        TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } => {
            if !defer {
                let _ = popup::show(tray.app_handle(), state);
                return;
            }
            let Some(token) = gate.lock().on_left_up() else { return };
            let app = tray.app_handle().clone();
            let state = state.clone();
            let gate = gate.clone();
            std::thread::spawn(move || {
                std::thread::sleep(double_click_interval());
                if !gate.lock().is_current(token) {
                    return;
                }
                let app_show = app.clone();
                let state_show = state.clone();
                let gate_show = gate.clone();
                let _ = app.run_on_main_thread(move || {
                    if !gate_show.lock().is_current(token) {
                        return;
                    }
                    let _ = popup::show(&app_show, &state_show);
                });
            });
        }
        TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => {
            if !defer {
                return;
            }
            gate.lock().on_double_click();
            command::run_quick_action(state);
        }
        _ => {}
    }
}

pub fn refresh(app: &AppHandle, state: &Arc<AppState>) {
    let png = icon_png(state);
    if let Ok(image) = Image::from_bytes(&png) {
        if let Some(tray) = app.tray_by_id("main") {
            let _ = tray.set_icon(Some(image));
            let settings = state.settings.lock().clone();
            let tip = format_tooltip(&state.last_response.lock(), &settings);
            let _ = tray.set_tooltip(Some(tip));
            if let Ok(menu) = build_menu(app, state) {
                let _ = tray.set_menu(Some(menu));
            }
        }
    }
}

fn parse_icon_field(raw: &str) -> (String, String) {
    match raw.split_once(':') {
        Some((name, mode)) => (name.to_string(), mode.to_string()),
        None => (raw.to_string(), "utilization".into()),
    }
}

fn displayed_quota_pct(entry: Option<&Value>) -> f64 {
    let Some(obj) = entry.and_then(Value::as_object) else { return 0.0 };
    let used = obj.get("utilization").and_then(Value::as_f64).unwrap_or(0.0);
    crate::formatting::display_pct(obj, used)
}

fn quota_time_pct(data: &serde_json::Map<String, Value>, field: &str) -> Option<f64> {
    let period = field_period(field)?;
    let resets = data.get(field).and_then(|v| v.get("resets_at")).and_then(Value::as_str).unwrap_or("");
    elapsed_pct(resets, period)
}

fn icon_png(state: &Arc<AppState>) -> Vec<u8> {
    let settings = state.settings.lock().clone();
    let data = state.last_response.lock().clone();
    let light = *state.light_taskbar.lock();
    if data.get("error").is_some() {
        return create_icon_png(0.0, 0.0, light, false, Some("!"), &settings, "utilization", "utilization", None, None);
    }
    let quota_keys: Vec<String> = data
        .iter()
        .filter(|(k, v)| {
            *k != "extra_usage"
                && v.get("utilization").and_then(Value::as_f64).is_some()
        })
        .map(|(k, _)| k.clone())
        .collect();
    let (mut top_field, top_mode) = settings
        .icon_fields
        .first()
        .map(|s| parse_icon_field(s))
        .unwrap_or_else(|| ("five_hour".into(), "utilization".into()));
    let (mut bottom_field, bottom_mode) = settings
        .icon_fields
        .get(1)
        .map(|s| parse_icon_field(s))
        .unwrap_or_else(|| ("seven_day".into(), "utilization".into()));
    if !data.contains_key(&top_field) && !quota_keys.is_empty() {
        top_field = quota_keys[0].clone();
    }
    if !data.contains_key(&bottom_field) && !quota_keys.is_empty() {
        bottom_field = if quota_keys.len() > 1 { quota_keys[1].clone() } else { quota_keys[0].clone() };
    }
    let top = displayed_quota_pct(data.get(&top_field));
    let bottom = displayed_quota_pct(data.get(&bottom_field));
    let time_pct_top = quota_time_pct(&data, &top_field);
    let time_pct_bottom = quota_time_pct(&data, &bottom_field);
    let extra = data.get("extra_usage").and_then(Value::as_object);
    let extra_available = extra
        .map(|e| e.get("is_enabled").and_then(Value::as_bool) == Some(true) && {
            let limit = e.get("monthly_limit").and_then(Value::as_f64).unwrap_or(0.0);
            let used = e.get("used_credits").and_then(Value::as_f64).unwrap_or(0.0);
            limit <= 0.0 || used < limit
        })
        .unwrap_or(false);
    create_icon_png(top, bottom, light, extra_available, None, &settings, &top_mode, &bottom_mode, time_pct_top, time_pct_bottom)
}

fn build_menu(app: &AppHandle, state: &Arc<AppState>) -> tauri::Result<Menu<tauri::Wry>> {
    let show = MenuItem::with_id(app, "show", t("menu_show"), true, None::<&str>)?;
    let settings = state.settings.lock().clone();
    let mut items: Vec<Box<dyn tauri::menu::IsMenuItem<tauri::Wry>>> = vec![Box::new(show)];
    if !settings.quick_action_command.is_empty() && !cfg!(windows) {
        items.push(Box::new(MenuItem::with_id(app, "quick", t("menu_quick_action"), true, None::<&str>)?));
    }
    items.push(Box::new(PredefinedMenuItem::separator(app)?));
    items.push(Box::new(CheckMenuItem::with_id(
        app,
        "autostart",
        t("autostart"),
        true,
        crate::platform::is_autostart(app).unwrap_or(false),
        None::<&str>,
    )?));
    let any_command = !settings.on_reset_command.is_empty()
        || !settings.on_startup_command.is_empty()
        || !settings.on_threshold_command.is_empty()
        || !settings.quick_action_command.is_empty();
    let mut test_owned: Vec<Box<dyn tauri::menu::IsMenuItem<tauri::Wry>>> = Vec::new();
    if any_command {
        let has_reset = !settings.on_reset_command.is_empty();
        let has_threshold = !settings.on_threshold_command.is_empty();
        let has_startup = !settings.on_startup_command.is_empty();
        let has_quick = !settings.quick_action_command.is_empty();
        test_owned.push(Box::new(MenuItem::with_id(app, "test_reset_5h", t("test_reset_5h"), has_reset, None::<&str>)?));
        test_owned.push(Box::new(MenuItem::with_id(app, "test_reset_7d", t("test_reset_7d"), has_reset, None::<&str>)?));
        test_owned.push(Box::new(MenuItem::with_id(app, "test_threshold_5h", t("test_threshold_5h"), has_threshold, None::<&str>)?));
        test_owned.push(Box::new(MenuItem::with_id(app, "test_threshold_7d", t("test_threshold_7d"), has_threshold, None::<&str>)?));
        test_owned.push(Box::new(MenuItem::with_id(app, "test_startup", t("test_startup"), has_startup, None::<&str>)?));
        test_owned.push(Box::new(MenuItem::with_id(app, "test_quick_action", t("test_quick_action"), has_quick, None::<&str>)?));
        let test_refs: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = test_owned.iter().map(|i| i.as_ref() as _).collect();
        items.push(Box::new(Submenu::with_id_and_items(app, "test_commands", t("test_commands"), true, &test_refs)?));
    }
    items.push(Box::new(MenuItem::with_id(app, "restart", t("restart"), true, None::<&str>)?));
    items.push(Box::new(PredefinedMenuItem::separator(app)?));
    let current = crate::sources::current_source_id(state);
    let mut source_owned: Vec<Box<dyn tauri::menu::IsMenuItem<tauri::Wry>>> = Vec::new();
    for src in sources::list_sources(&settings) {
        let id = format!("src:{}", src.id);
        source_owned.push(Box::new(CheckMenuItem::with_id(app, &id, &src.label, true, src.id == current, None::<&str>)?));
    }
    if source_owned.is_empty() {
        source_owned.push(Box::new(MenuItem::with_id(app, "src:none", t("no_providers"), false, None::<&str>)?));
    }
    let refs: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = source_owned.iter().map(|i| i.as_ref() as _).collect();
    let submenu = Submenu::with_id_and_items(app, "sources", t("menu_providers"), true, &refs)?;
    items.push(Box::new(submenu));
    items.push(Box::new(MenuItem::with_id(app, "quit", t("quit"), true, None::<&str>)?));
    let item_refs: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = items.iter().map(|i| i.as_ref() as _).collect();
    Menu::with_items(app, &item_refs)
}

fn handle_menu(app: &AppHandle, state: &Arc<AppState>, id: &str) {
    match id {
        "show" => { let _ = popup::show(app, state); }
        "quick" => command::run_quick_action(state),
        "test_reset_5h" | "test_reset_7d" | "test_threshold_5h" | "test_threshold_7d"
        | "test_startup" | "test_quick_action" => command::run_test_command(id, state),
        "autostart" => {
            let enabled = crate::platform::is_autostart(app).unwrap_or(false);
            let _ = crate::platform::set_autostart(app, !enabled);
        }
        "restart" => app.restart(),
        "quit" => {
            *state.running.lock() = false;
            app.exit(0);
        }
        other if other.starts_with("src:") => {
            let sid = other.trim_start_matches("src:");
            if sid != "none" {
                sources::select_source(state, sid.to_string());
                crate::poll::force_update(app.clone(), state.clone());
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod click_gate_tests {
    use super::ClickGate;

    #[test]
    fn single_click_issues_token() {
        let mut g = ClickGate::default();
        assert_eq!(g.on_left_up(), Some(1));
        assert!(g.is_current(1));
    }

    #[test]
    fn double_click_cancels_pending_and_swallows_trailing_up() {
        let mut g = ClickGate::default();
        let token = g.on_left_up().unwrap();
        g.on_double_click();
        assert!(!g.is_current(token));
        assert_eq!(g.on_left_up(), None);
        assert_eq!(g.on_left_up(), Some(3));
    }

    #[test]
    fn second_click_before_interval_replaces_token() {
        let mut g = ClickGate::default();
        let first = g.on_left_up().unwrap();
        let second = g.on_left_up().unwrap();
        assert!(!g.is_current(first));
        assert!(g.is_current(second));
    }
}
