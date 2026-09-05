use crate::error::Result;
use std::process::Command;

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
    #[cfg(not(windows))]
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
    #[cfg(not(windows))]
    {
        unix::idle_seconds()
    }
}

pub fn is_workstation_locked() -> bool {
    #[cfg(windows)]
    {
        win::is_workstation_locked()
    }
    #[cfg(not(windows))]
    {
        unix::is_workstation_locked()
    }
}

pub fn light_taskbar() -> bool {
    #[cfg(windows)]
    {
        win::light_taskbar()
    }
    #[cfg(not(windows))]
    {
        false
    }
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
    #[cfg(not(windows))]
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
/// (taskbar left/top/right/bottom). Linux: primary work-area top-right.
pub fn popup_anchor(app: &tauri::AppHandle, logical_width: f64, logical_height: f64) -> Option<(f64, f64)> {
    #[cfg(windows)]
    {
        let _ = app;
        win::tray_anchor(logical_width, logical_height)
    }
    #[cfg(not(windows))]
    {
        linux_anchor(app, logical_width, logical_height)
    }
}

#[cfg(not(windows))]
fn linux_anchor(app: &tauri::AppHandle, logical_width: f64, logical_height: f64) -> Option<(f64, f64)> {
    use tauri::Manager;
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
    use std::process::Command;
    use std::sync::Once;

    const IDLE_DEST: &str = "org.gnome.Mutter.IdleMonitor";
    const IDLE_PATH: &str = "/org/gnome/Mutter/IdleMonitor/Core";
    const IDLE_METHOD: &str = "org.gnome.Mutter.IdleMonitor.GetIdletime";
    const LOCK_DEST: &str = "org.gnome.ScreenSaver";
    const LOCK_PATH: &str = "/org/gnome/ScreenSaver";
    const LOCK_METHOD: &str = "org.gnome.ScreenSaver.GetActive";

    pub fn system_time_format() -> String {
        time_format_from_pattern(&nl_time_fmt()).into()
    }

    pub fn idle_seconds() -> f64 {
        match session_call(IDLE_DEST, IDLE_PATH, IDLE_METHOD).and_then(|s| parse_dbus_u64(&s)) {
            Some(ms) => ms as f64 / 1000.0,
            None => 0.0,
        }
    }

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

    pub(super) fn parse_dbus_u64(s: &str) -> Option<u64> {
        s.split_whitespace().find_map(|tok| {
            tok.trim_matches(|c: char| !c.is_ascii_digit()).parse().ok()
        })
    }

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

    #[cfg(not(windows))]
    #[test]
    fn linux_dbus_parsers() {
        assert_eq!(super::unix::parse_dbus_u64("uint64 7500"), Some(7500));
        assert_eq!(super::unix::parse_dbus_u64("(uint64 7500,)"), Some(7500));
        assert_eq!(super::unix::parse_dbus_bool("boolean true"), Some(true));
        assert_eq!(super::unix::parse_dbus_bool("boolean false"), Some(false));
        assert_eq!(super::unix::parse_dbus_bool(""), None);
    }
}
