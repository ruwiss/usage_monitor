use crate::dismiss;
use crate::error::Result;
use crate::formatting::snapshot_to_popup;
use crate::platform;
use crate::state::AppState;
use crate::types::PopupInit;
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;
use tauri::{Emitter, LogicalPosition, LogicalSize, Manager};

const POPUP_WIDTH: f64 = 340.0;

pub fn hide(app: &tauri::AppHandle, state: &Arc<AppState>) -> Result<()> {
    *state.popup_shown.lock() = false;
    *state.popup_pinned.lock() = false;
    *state.popup_moved.lock() = false;
    *state.popup_closed_at.lock() = Some(Instant::now());
    dismiss::stop();
    if let Some(w) = app.get_webview_window("popup") {
        let _ = w.hide();
    }
    let _ = app.emit("popup://reset-pin", ());
    Ok(())
}

pub fn toggle(app: &tauri::AppHandle, state: &Arc<AppState>) -> Result<()> {
    if *state.popup_shown.lock() {
        hide(app, state)
    } else {
        show(app, state)
    }
}

pub fn show(app: &tauri::AppHandle, state: &Arc<AppState>) -> Result<()> {
    if *state.popup_shown.lock() {
        return Ok(());
    }
    if let Some(closed) = *state.popup_closed_at.lock() {
        if closed.elapsed() < std::time::Duration::from_millis(150) {
            return Ok(());
        }
    }
    *state.popup_pinned.lock() = false;
    *state.popup_moved.lock() = false;
    if let Some(w) = app.get_webview_window("popup") {
        let height = current_height(&w);
        move_to_anchor(app, &w, height);
        let _ = w.show();
        let _ = w.set_focus();
    }
    *state.popup_shown.lock() = true;
    let _ = app.emit("popup://reset-pin", ());
    dismiss::start(app, state);
    crate::poll::force_update(app.clone(), state.clone());
    Ok(())
}

pub fn open_dashboard() -> Result<()> {
    Ok(())
}

pub fn begin_drag(app: &tauri::AppHandle, state: &Arc<AppState>) -> Result<bool> {
    if !*state.popup_pinned.lock() {
        return Ok(false);
    }
    if let Some(w) = app.get_webview_window("popup") {
        let _ = w.start_dragging();
        return Ok(true);
    }
    Ok(false)
}

pub fn drag(_app: &tauri::AppHandle, state: &Arc<AppState>) -> Result<bool> {
    if !*state.popup_pinned.lock() {
        return Ok(false);
    }
    *state.popup_moved.lock() = true;
    Ok(true)
}

pub fn end_drag(_app: &tauri::AppHandle, _state: &Arc<AppState>) -> Result<()> {
    Ok(())
}

const MAX_SCREEN_RATIO: f64 = 0.9;

pub fn apply_height(app: &tauri::AppHandle, state: &Arc<AppState>, height: i32) -> Result<()> {
    if let Some(w) = app.get_webview_window("popup") {
        let h = clamp_popup_height(height.max(1) as f64, max_monitor_height(&w));
        let _ = w.set_size(LogicalSize::new(POPUP_WIDTH, h));
        if *state.popup_shown.lock() {
            let keep = *state.popup_pinned.lock() && *state.popup_moved.lock();
            if !keep {
                move_to_anchor(app, &w, h);
            }
        }
    }
    Ok(())
}

fn clamp_popup_height(requested: f64, work_height: f64) -> f64 {
    let max = (work_height * MAX_SCREEN_RATIO).floor().max(1.0);
    requested.max(1.0).min(max)
}

fn max_monitor_height(w: &tauri::WebviewWindow) -> f64 {
    let mon = w
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| w.primary_monitor().ok().flatten());
    let Some(mon) = mon else {
        return f64::MAX;
    };
    let scale = mon.scale_factor();
    mon.work_area().size.height as f64 / scale
}

fn current_height(w: &tauri::WebviewWindow) -> f64 {
    let scale = w.scale_factor().unwrap_or(1.0);
    w.inner_size()
        .ok()
        .map(|s| s.height as f64 / scale)
        .unwrap_or(400.0)
}

fn move_to_anchor(app: &tauri::AppHandle, w: &tauri::WebviewWindow, height: f64) {
    if let Some((x, y)) = platform::popup_anchor(app, POPUP_WIDTH, height) {
        let _ = w.set_position(LogicalPosition::new(x, y));
    }
}

pub fn init_payload(state: &Arc<AppState>) -> Result<PopupInit> {
    let settings = state.settings.lock().clone();
    let colors = json!({
        "bg": settings.bg,
        "fg": settings.fg,
        "fg_dim": settings.fg_dim,
        "fg_heading": settings.fg_heading,
        "fg_link": settings.fg_link,
        "bar_bg": settings.bar_bg,
        "bar_fg": settings.bar_fg,
        "bar_fg_warn": settings.bar_fg_warn,
        "bar_divider": settings.bar_divider,
        "bar_marker": settings.bar_marker,
    });
    let t = json!({
        "title": crate::i18n::t("popup_title"),
        "account": crate::i18n::t("account"),
        "email": crate::i18n::t("email"),
        "plan": crate::i18n::t("plan"),
        "usage": crate::i18n::t("usage"),
        "extra_usage": crate::i18n::t("extra_usage"),
        "pin_popup": crate::i18n::t("pin_popup"),
        "unpin_popup": crate::i18n::t("unpin_popup"),
        "status_updated_s": crate::i18n::t("status_updated_s"),
        "status_updated": crate::i18n::t("status_updated"),
        "status_next_update": crate::i18n::t("status_next_update"),
        "status_refreshing": crate::i18n::t("status_refreshing"),
        "duration_hm": crate::i18n::t("duration_hm"),
        "duration_m": crate::i18n::t("duration_m"),
        "duration_s": crate::i18n::t("duration_s"),
    });
    let data = snapshot_to_popup(
        &state.cache.usage(),
        state.cache.profile().as_ref(),
        state.cache.last_success_time(),
        *state.next_poll_time.lock(),
        state.cache.refreshing(),
        state.cache.last_error().as_deref(),
        &settings,
    );
    Ok(PopupInit {
        colors,
        t,
        app_version: env!("CARGO_PKG_VERSION").into(),
        compact_hide: settings.compact_hide.clone(),
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::clamp_popup_height;

    #[test]
    fn popup_height_caps_at_90_percent_of_work_area() {
        assert_eq!(clamp_popup_height(2000.0, 1000.0), 900.0);
        assert_eq!(clamp_popup_height(400.0, 1000.0), 400.0);
        assert_eq!(clamp_popup_height(0.0, 1000.0), 1.0);
    }
}
