use crate::error::Result;
use std::path::PathBuf;
use std::process::Command;

#[cfg(not(windows))]
use tauri::Manager;

#[cfg(windows)]
mod win {
    use windows_sys::Win32::Foundation::FALSE;
    use windows_sys::Win32::Globalization::{
        GetLocaleInfoEx, GetUserDefaultLocaleName, LOCALE_RETURN_NUMBER,
        LOCALE_SENGLISHCOUNTRYNAME, LOCALE_SENGLISHLANGUAGENAME,
    };
    use windows_sys::Win32::System::StationsAndDesktops::{CloseDesktop, OpenInputDesktop};
    use windows_sys::Win32::System::SystemInformation::GetTickCount;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    use windows_sys::Win32::UI::HiDpi::{GetDpiForSystem, GetDpiForWindow};
    use windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW;

    const LOCALE_ITIME: u32 = 0x00000023;

    pub fn system_time_format() -> String {
        let mut value: u32 = 1;
        let chars = unsafe {
            GetLocaleInfoEx(
                std::ptr::null(),
                LOCALE_ITIME | LOCALE_RETURN_NUMBER,
                &mut value as *mut u32 as *mut u16,
                2,
            )
        };
        if chars == 0 {
            return "24h".into();
        }
        if value == 1 { "24h" } else { "12h" }.into()
    }

    pub fn idle_seconds() -> f64 {
        let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        let ok = unsafe { GetLastInputInfo(&mut info) };
        if ok == FALSE {
            return 0.0;
        }
        let ticks = unsafe { GetTickCount() };
        ticks.wrapping_sub(info.dwTime) as f64 / 1000.0
    }

    pub fn is_workstation_locked() -> bool {
        // OpenInputDesktop fails only on the secure desktop (Win+L). Screensaver is not away.
        unsafe {
            let hdesk = OpenInputDesktop(0, FALSE, 0);
            if hdesk.is_null() {
                return true;
            }
            CloseDesktop(hdesk);
            false
        }
    }

    pub fn os_locale() -> String {
        let mut buf = [0u16; 85];
        let n = unsafe { GetUserDefaultLocaleName(buf.as_mut_ptr(), buf.len() as i32) };
        if n > 1 {
            return String::from_utf16_lossy(&buf[..n as usize - 1]);
        }
        let lang = locale_info_string(LOCALE_SENGLISHLANGUAGENAME);
        let country = locale_info_string(LOCALE_SENGLISHCOUNTRYNAME);
        if lang.is_empty() {
            String::new()
        } else if country.is_empty() {
            lang
        } else {
            format!("{lang}_{country}")
        }
    }

    fn locale_info_string(lc_type: u32) -> String {
        let mut buf = [0u16; 85];
        let n = unsafe { GetLocaleInfoEx(std::ptr::null(), lc_type, buf.as_mut_ptr(), buf.len() as i32) };
        if n > 1 {
            String::from_utf16_lossy(&buf[..n as usize - 1])
        } else {
            String::new()
        }
    }

    pub fn light_taskbar() -> bool {
        use windows_sys::Win32::System::Registry::{
            RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_DWORD,
        };
        const KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0";
        const VAL: &str = "SystemUsesLightTheme\0";
        let mut hkey: HKEY = std::ptr::null_mut();
        let wide_key: Vec<u16> = KEY.encode_utf16().collect();
        let wide_val: Vec<u16> = VAL.encode_utf16().collect();
        let status = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, wide_key.as_ptr(), 0, KEY_READ, &mut hkey) };
        if status != 0 {
            return false;
        }
        let mut data: u32 = 0;
        let mut size = 4u32;
        let mut ty = REG_DWORD;
        let q = unsafe {
            RegQueryValueExW(
                hkey,
                wide_val.as_ptr(),
                std::ptr::null_mut(),
                &mut ty,
                &mut data as *mut u32 as *mut u8,
                &mut size,
            )
        };
        q == 0 && data == 1
    }


    pub fn tray_anchor(logical_width: f64, logical_height: f64) -> Option<(f64, f64)> {
        const MARGIN: i32 = 12;
        const BASELINE: f64 = 96.0;
        unsafe {
            let class: Vec<u16> = "Shell_TrayWnd\0".encode_utf16().collect();
            let tray = FindWindowW(class.as_ptr(), std::ptr::null());
            let hmon = MonitorFromWindow(tray, MONITOR_DEFAULTTONEAREST);
            if hmon.is_null() {
                return None;
            }
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                rcMonitor: std::mem::zeroed(),
                rcWork: std::mem::zeroed(),
                dwFlags: 0,
            };
            if GetMonitorInfoW(hmon, &mut info) == 0 {
                return None;
            }
            let dpi = {
                let window_dpi = GetDpiForWindow(tray);
                if window_dpi == 0 { GetDpiForSystem() } else { window_dpi }
            };
            let scale = dpi as f64 / BASELINE;
            let physical_width = (logical_width * scale) as i32;
            let physical_height = (logical_height * scale) as i32;
            let mon = info.rcMonitor;
            let work = info.rcWork;
            let x = if work.left > mon.left {
                work.left + MARGIN
            } else {
                work.right - physical_width - MARGIN
            };
            let y = if work.top > mon.top {
                work.top + MARGIN
            } else {
                work.bottom - physical_height - MARGIN
            };
            Some((x as f64 / scale, y as f64 / scale))
        }
    }
}

pub fn os_locale() -> String {
    #[cfg(windows)]
    {
        win::os_locale()
    }
    #[cfg(target_os = "macos")]
    {
        macos::os_locale()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        String::new()
    }
}

pub fn system_time_format() -> String {
    #[cfg(windows)]
    {
        win::system_time_format()
    }
    #[cfg(not(windows))]
    {
        unix::system_time_format()
    }
}

pub fn idle_seconds() -> f64 {
    #[cfg(windows)]
    {
        win::idle_seconds()
    }
    #[cfg(target_os = "macos")]
    {
        macos::idle_seconds()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        unix::idle_seconds()
    }
}

pub fn is_workstation_locked() -> bool {
    #[cfg(windows)]
    {
        win::is_workstation_locked()
    }
    #[cfg(target_os = "macos")]
    {
        macos::is_workstation_locked()
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        unix::is_workstation_locked()
    }
}

pub fn light_taskbar() -> bool {
    #[cfg(windows)]
    {
        win::light_taskbar()
    }
    #[cfg(target_os = "macos")]
    {
        macos::light_menubar()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        false
    }
}

/// Prepend Homebrew / user bin dirs so GUI-launched apps find `claude`, `omp`, etc.
pub fn ensure_gui_path() {
    #[cfg(target_os = "macos")]
    {
        macos::ensure_gui_path();
    }
}

/// Directory of `Usage Monitor.app` when running from a bundle.
pub fn macos_bundle_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        macos::bundle_dir()
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// CI builds are ad-hoc signed. Downloaded updates pick up Gatekeeper quarantine
/// (`com.apple.quarantine`), which macOS reports as a damaged app. Same fix as
/// `xattr -cr "/Applications/Usage Monitor.app"`, without sudo.
pub fn clear_macos_quarantine() {
    #[cfg(target_os = "macos")]
    {
        macos::clear_quarantine();
    }
}

pub fn event_command_cwd() -> PathBuf {
    if cfg!(debug_assertions) {
        return PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    }
    if let Some(dir) = macos_bundle_dir().and_then(|b| b.parent().map(|p| p.to_path_buf())) {
        return dir;
    }
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."))
}

pub fn set_autostart(app: &tauri::AppHandle, enable: bool) -> Result<()> {
    use tauri_plugin_autostart::ManagerExt;
    let mgr = app.autolaunch();
    if enable {
        mgr.enable().map_err(|e| e.to_string())?;
    } else {
        mgr.disable().map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub fn is_autostart(app: &tauri::AppHandle) -> Result<bool> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string().into())
}

pub fn no_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = cmd;
}

pub fn show_error_box(message: &str, title: &str) {
    let message: String = message.chars().take(2000).collect();
    let message = message.as_str();
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};
        let to_wide = |s: &str| s.encode_utf16().chain(std::iter::once(0)).collect::<Vec<u16>>();
        let msg = to_wide(message);
        let ttl = to_wide(title);
        unsafe {
            MessageBoxW(std::ptr::null_mut(), msg.as_ptr(), ttl.as_ptr(), MB_ICONERROR | MB_OK);
        }
        return;
    }
    #[cfg(target_os = "macos")]
    {
        macos::show_error_box(message, title);
        return;
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut cmd = Command::new("zenity");
        cmd.args(["--error", "--title", title, "--text", message, "--no-wrap"]);
        no_window(&mut cmd);
        if cmd.status().map(|s| s.success()).unwrap_or(false) {
            return;
        }
        eprintln!("{title}: {message}");
    }
}

/// Logical (x, y) for the popup, matching the original Python hosts.
///
/// Windows: work-area corner of the monitor that owns `Shell_TrayWnd`
/// (taskbar left/top/right/bottom). macOS: under the status item (menu bar).
/// Linux: primary work-area top-right.
pub fn popup_anchor(app: &tauri::AppHandle, logical_width: f64, logical_height: f64) -> Option<(f64, f64)> {
    #[cfg(windows)]
    {
        let _ = app;
        win::tray_anchor(logical_width, logical_height)
    }
    #[cfg(target_os = "macos")]
    {
        let rect = app
            .try_state::<std::sync::Arc<crate::state::AppState>>()
            .and_then(|s| *s.last_tray_rect.lock());
        macos::tray_anchor(app, rect, logical_width, logical_height)
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        linux_anchor(app, logical_width, logical_height)
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
fn linux_anchor(app: &tauri::AppHandle, logical_width: f64, logical_height: f64) -> Option<(f64, f64)> {
    const MARGIN: f64 = 8.0;
    let mon = app.primary_monitor().ok().flatten()?;
    let work = mon.work_area();
    let scale = mon.scale_factor();
    let x = work.position.x as f64 / scale + work.size.width as f64 / scale - logical_width - MARGIN;
    let y = work.position.y as f64 / scale + MARGIN;
    let _ = logical_height;
    Some((x, y))
}

#[cfg(not(windows))]
mod unix {
    use std::ffi::CStr;
    #[cfg(not(target_os = "macos"))]
    use std::process::Command;
    use std::sync::Once;

    #[cfg(not(target_os = "macos"))]
    const IDLE_DEST: &str = "org.gnome.Mutter.IdleMonitor";
    #[cfg(not(target_os = "macos"))]
    const IDLE_PATH: &str = "/org/gnome/Mutter/IdleMonitor/Core";
    #[cfg(not(target_os = "macos"))]
    const IDLE_METHOD: &str = "org.gnome.Mutter.IdleMonitor.GetIdletime";
    #[cfg(not(target_os = "macos"))]
    const LOCK_DEST: &str = "org.gnome.ScreenSaver";
    #[cfg(not(target_os = "macos"))]
    const LOCK_PATH: &str = "/org/gnome/ScreenSaver";
    #[cfg(not(target_os = "macos"))]
    const LOCK_METHOD: &str = "org.gnome.ScreenSaver.GetActive";

    pub fn system_time_format() -> String {
        time_format_from_pattern(&nl_time_fmt()).into()
    }

    #[cfg(not(target_os = "macos"))]
    pub fn idle_seconds() -> f64 {
        match session_call(IDLE_DEST, IDLE_PATH, IDLE_METHOD).and_then(|s| parse_dbus_u64(&s)) {
            Some(ms) => ms as f64 / 1000.0,
            None => 0.0,
        }
    }

    #[cfg(not(target_os = "macos"))]
    pub fn is_workstation_locked() -> bool {
        session_call(LOCK_DEST, LOCK_PATH, LOCK_METHOD)
            .and_then(|s| parse_dbus_bool(&s))
            .unwrap_or(false)
    }

    fn nl_time_fmt() -> String {
        static INIT: Once = Once::new();
        INIT.call_once(|| unsafe {
            libc::setlocale(libc::LC_TIME, b"\0".as_ptr().cast());
        });
        unsafe {
            let p = libc::nl_langinfo(libc::T_FMT);
            if p.is_null() {
                return String::new();
            }
            CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn session_call(dest: &str, path: &str, method: &str) -> Option<String> {
        // ponytail: dbus-send vs libdbus. Upgrade if we already spawn a session bus elsewhere.
        let out = Command::new("dbus-send")
            .args([
                "--session",
                "--type=method_call",
                "--print-reply=literal",
                "--reply-timeout=2000",
                &format!("--dest={dest}"),
                path,
                method,
            ])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&out.stdout);
        let trimmed = text.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    pub(super) fn time_format_from_pattern(pattern: &str) -> &'static str {
        if pattern.contains("%p") || pattern.contains("%I") {
            "12h"
        } else {
            "24h"
        }
    }

    #[cfg(not(target_os = "macos"))]
    pub(super) fn parse_dbus_u64(s: &str) -> Option<u64> {
        s.split_whitespace().find_map(|tok| {
            tok.trim_matches(|c: char| !c.is_ascii_digit()).parse().ok()
        })
    }

    #[cfg(not(target_os = "macos"))]
    pub(super) fn parse_dbus_bool(s: &str) -> Option<bool> {
        let lower = s.to_ascii_lowercase();
        if lower.split_whitespace().any(|w| w.trim_matches(|c: char| !c.is_ascii_alphabetic()) == "true") {
            Some(true)
        } else if lower.split_whitespace().any(|w| w.trim_matches(|c: char| !c.is_ascii_alphabetic()) == "false") {
            Some(false)
        } else {
            None
        }
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::*;
    use std::ffi::CString;
    use std::os::raw::c_void;
    use std::path::PathBuf;
    use std::ptr;

    const K_CF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const K_CG_EVENT_SOURCE_STATE_COMBINED_SESSION_STATE: u32 = 0;
    const K_CG_ANY_INPUT_EVENT_TYPE: u32 = u32::MAX;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGEventSourceSecondsSinceLastEventType(state_id: u32, event_type: u32) -> f64;
        fn CGSessionCopyCurrentDictionary() -> *mut c_void;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFRelease(cf: *mut c_void);
        fn CFDictionaryGetValue(the_dict: *mut c_void, key: *const c_void) -> *const c_void;
        fn CFGetTypeID(cf: *const c_void) -> usize;
        fn CFBooleanGetTypeID() -> usize;
        fn CFBooleanGetValue(boolean: *const c_void) -> u8;
        fn CFStringCreateWithCString(
            alloc: *mut c_void,
            c_str: *const i8,
            encoding: u32,
        ) -> *mut c_void;
    }

    pub fn os_locale() -> String {
        defaults_read("AppleLocale").unwrap_or_default()
    }

    pub fn light_menubar() -> bool {
        match defaults_read("AppleInterfaceStyle") {
            Some(s) if s.eq_ignore_ascii_case("dark") => false,
            _ => true,
        }
    }

    pub fn idle_seconds() -> f64 {
        let secs = unsafe {
            CGEventSourceSecondsSinceLastEventType(
                K_CG_EVENT_SOURCE_STATE_COMBINED_SESSION_STATE,
                K_CG_ANY_INPUT_EVENT_TYPE,
            )
        };
        if secs.is_finite() && secs >= 0.0 {
            secs
        } else {
            0.0
        }
    }

    pub fn is_workstation_locked() -> bool {
        unsafe {
            let dict = CGSessionCopyCurrentDictionary();
            if dict.is_null() {
                return false;
            }
            let key = cfstr("CGSSessionScreenIsLocked");
            let val = if key.is_null() {
                ptr::null()
            } else {
                CFDictionaryGetValue(dict, key)
            };
            let locked = !val.is_null()
                && CFGetTypeID(val) == CFBooleanGetTypeID()
                && CFBooleanGetValue(val) != 0;
            if !key.is_null() {
                CFRelease(key);
            }
            CFRelease(dict);
            locked
        }
    }

    pub fn show_error_box(message: &str, title: &str) {
        let script = format!(
            "display alert {} message {} as critical buttons {{\"OK\"}} default button 1",
            applescript_string(title),
            applescript_string(message)
        );
        let mut cmd = Command::new("osascript");
        cmd.args(["-e", &script]);
        no_window(&mut cmd);
        if cmd.status().map(|s| s.success()).unwrap_or(false) {
            return;
        }
        eprintln!("{title}: {message}");
    }

    pub fn ensure_gui_path() {
        let mut parts: Vec<String> = std::env::var("PATH")
            .unwrap_or_default()
            .split(':')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        let home = dirs::home_dir().unwrap_or_default();
        let extras = [
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/opt/homebrew/sbin"),
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/usr/local/sbin"),
            home.join(".local/bin"),
            home.join(".cargo/bin"),
            home.join("Library/pnpm"),
            home.join(".npm-global/bin"),
            home.join(".volta/bin"),
        ];
        for dir in extras.into_iter().rev() {
            if !dir.is_dir() {
                continue;
            }
            let s = dir.to_string_lossy().into_owned();
            if !parts.iter().any(|p| p == &s) {
                parts.insert(0, s);
            }
        }
        std::env::set_var("PATH", parts.join(":"));
    }

    pub fn bundle_dir() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let macos_dir = exe.parent()?;
        if macos_dir.file_name()?.to_str()? != "MacOS" {
            return None;
        }
        let contents = macos_dir.parent()?;
        if contents.file_name()?.to_str()? != "Contents" {
            return None;
        }
        contents.parent().map(PathBuf::from)
    }

    pub fn clear_quarantine() {
        let mut paths = Vec::new();
        if let Some(bundle) = bundle_dir() {
            paths.push(bundle);
        }
        let apps = PathBuf::from("/Applications/Usage Monitor.app");
        if apps.is_dir() && !paths.contains(&apps) {
            paths.push(apps);
        }
        for path in paths {
            let mut drop_flag = Command::new("xattr");
            drop_flag.args(["-d", "-r", "com.apple.quarantine"]).arg(&path);
            no_window(&mut drop_flag);
            let _ = drop_flag.status();
            let mut clear_all = Command::new("xattr");
            clear_all.args(["-cr"]).arg(&path);
            no_window(&mut clear_all);
            let _ = clear_all.status();
        }
    }

    pub fn tray_anchor(
        app: &tauri::AppHandle,
        rect: Option<[f64; 4]>,
        logical_width: f64,
        logical_height: f64,
    ) -> Option<(f64, f64)> {
        const GAP: f64 = 6.0;
        const MARGIN: f64 = 8.0;
        let mon = app.primary_monitor().ok().flatten()?;
        let scale = mon.scale_factor();
        let work = mon.work_area();
        let work_x = work.position.x as f64 / scale;
        let work_y = work.position.y as f64 / scale;
        let work_w = work.size.width as f64 / scale;
        let work_h = work.size.height as f64 / scale;
        let (x, y) = if let Some([icon_x, icon_y, icon_w, icon_h]) = rect {
            (
                icon_x + icon_w / 2.0 - logical_width / 2.0,
                icon_y + icon_h + GAP,
            )
        } else {
            (
                work_x + work_w - logical_width - MARGIN,
                work_y + MARGIN,
            )
        };
        Some(clamp_popup(
            x,
            y,
            logical_width,
            logical_height,
            work_x,
            work_y,
            work_w,
            work_h,
        ))
    }

    pub(super) fn clamp_popup(
        x: f64,
        y: f64,
        width: f64,
        height: f64,
        work_x: f64,
        work_y: f64,
        work_w: f64,
        work_h: f64,
    ) -> (f64, f64) {
        const MARGIN: f64 = 8.0;
        let max_x = (work_x + work_w - width - MARGIN).max(work_x + MARGIN);
        let max_y = (work_y + work_h - height - MARGIN).max(work_y);
        (
            x.clamp(work_x + MARGIN, max_x),
            y.clamp(work_y, max_y),
        )
    }

    fn defaults_read(key: &str) -> Option<String> {
        let out = Command::new("defaults")
            .args(["read", "-g", key])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }

    fn applescript_string(s: &str) -> String {
        format!(
            "\"{}\"",
            s.replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace('\n', " ")
        )
    }

    fn cfstr(s: &str) -> *mut c_void {
        let Ok(c) = CString::new(s) else {
            return ptr::null_mut();
        };
        unsafe { CFStringCreateWithCString(ptr::null_mut(), c.as_ptr(), K_CF_STRING_ENCODING_UTF8) }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    #[test]
    fn windows_os_locale_not_empty() {
        let loc = super::os_locale();
        assert!(!loc.is_empty(), "GetUserDefaultLocaleName/GetLocaleInfoEx returned empty");
    }

    #[cfg(windows)]
    #[test]
    fn unlocked_session_is_not_away() {
        assert!(
            !super::is_workstation_locked(),
            "OpenInputDesktop must succeed on the interactive desktop; screensaver is not lock"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn linux_time_format_from_pattern() {
        assert_eq!(super::unix::time_format_from_pattern("%H:%M:%S"), "24h");
        assert_eq!(super::unix::time_format_from_pattern("%I:%M:%S %p"), "12h");
        assert_eq!(super::unix::time_format_from_pattern("%I:%M"), "12h");
        assert_eq!(super::unix::time_format_from_pattern(""), "24h");
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn linux_dbus_parsers() {
        assert_eq!(super::unix::parse_dbus_u64("uint64 7500"), Some(7500));
        assert_eq!(super::unix::parse_dbus_u64("(uint64 7500,)"), Some(7500));
        assert_eq!(super::unix::parse_dbus_bool("boolean true"), Some(true));
        assert_eq!(super::unix::parse_dbus_bool("boolean false"), Some(false));
        assert_eq!(super::unix::parse_dbus_bool(""), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_os_locale_not_empty() {
        let loc = super::os_locale();
        assert!(!loc.is_empty(), "AppleLocale should be readable, got {loc:?}");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_idle_is_finite() {
        let idle = super::idle_seconds();
        assert!(idle.is_finite() && idle >= 0.0, "idle_seconds={idle}");
        assert!(
            !super::is_workstation_locked(),
            "interactive session should not report lock"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_clamp_popup_stays_in_work_area() {
        let (x, y) = super::macos::clamp_popup(5000.0, -10.0, 340.0, 400.0, 0.0, 25.0, 1440.0, 875.0);
        assert!(x + 340.0 <= 1440.0);
        assert!(x >= 8.0);
        assert!(y >= 25.0);
    }

    #[test]
    fn clear_macos_quarantine_is_safe() {
        super::clear_macos_quarantine();
    }
}
