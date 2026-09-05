use crate::popup;
use crate::state::AppState;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager};

static ENABLED: AtomicBool = AtomicBool::new(false);
static HWND: AtomicIsize = AtomicIsize::new(0);
static PUMP_TID: AtomicU32 = AtomicU32::new(0);
static SETTLE_GEN: AtomicU64 = AtomicU64::new(0);

pub fn sync(state: &AppState) {
    let on = *state.popup_shown.lock() && !*state.popup_pinned.lock();
    ENABLED.store(on, Ordering::SeqCst);
}

pub fn start(app: &AppHandle, state: &Arc<AppState>) {
    sync(state);
    #[cfg(windows)]
    win::start(app.clone(), state.clone());
    let _ = (app, state);
}

pub fn stop() {
    ENABLED.store(false, Ordering::SeqCst);
    #[cfg(windows)]
    win::stop();
}

#[cfg(windows)]
fn popup_hwnd(app: &AppHandle) -> isize {
    let Some(w) = app.get_webview_window("popup") else {
        return 0;
    };
    match w.hwnd() {
        Ok(h) => h.0 as isize,
        Err(_) => 0,
    }
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::mem::zeroed;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Threading::GetCurrentThreadId;
    use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetAncestor, GetForegroundWindow, GetMessageW, GetWindowRect, IsChild,
        PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx, EVENT_SYSTEM_FOREGROUND,
        GA_ROOTOWNER, KBDLLHOOKSTRUCT, MSLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WH_MOUSE_LL,
        WINEVENT_SKIPOWNPROCESS, WM_KEYDOWN, WM_LBUTTONDOWN, WM_QUIT,
    };

    const VK_ESCAPE: u32 = 0x1B;
    const SETTLE_MS: u64 = 200;

    pub fn start(app: AppHandle, state: Arc<AppState>) {
        stop();
        HWND.store(super::popup_hwnd(&app), Ordering::SeqCst);
        std::thread::spawn(move || pump(app, state));
    }

    pub fn stop() {
        SETTLE_GEN.fetch_add(1, Ordering::SeqCst);
        let tid = PUMP_TID.swap(0, Ordering::SeqCst);
        if tid != 0 {
            unsafe {
                PostThreadMessageW(tid, WM_QUIT, 0, 0);
            }
        }
    }

    fn owns(hwnd: HWND) -> bool {
        let popup = HWND.load(Ordering::SeqCst) as HWND;
        if popup.is_null() || hwnd.is_null() {
            return false;
        }
        if hwnd == popup {
            return true;
        }
        unsafe {
            if IsChild(popup, hwnd) != 0 {
                return true;
            }
            GetAncestor(hwnd, GA_ROOTOWNER) == popup
        }
    }

    fn request_close() {
        if !ENABLED.load(Ordering::SeqCst) {
            return;
        }
        let tid = PUMP_TID.load(Ordering::SeqCst);
        if tid != 0 {
            unsafe {
                PostThreadMessageW(tid, WM_QUIT, 0, 0);
            }
        }
    }

    unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 && wparam == WM_LBUTTONDOWN as WPARAM && ENABLED.load(Ordering::SeqCst) {
            let popup = HWND.load(Ordering::SeqCst) as HWND;
            if !popup.is_null() {
                let info = &*(lparam as *const MSLLHOOKSTRUCT);
                let mut rect = RECT {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                };
                if GetWindowRect(popup, &mut rect) != 0 {
                    let x = info.pt.x;
                    let y = info.pt.y;
                    if x < rect.left || x > rect.right || y < rect.top || y > rect.bottom {
                        request_close();
                    }
                }
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }

    unsafe extern "system" fn kb_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 && wparam == WM_KEYDOWN as WPARAM && ENABLED.load(Ordering::SeqCst) {
            let info = &*(lparam as *const KBDLLHOOKSTRUCT);
            if info.vkCode == VK_ESCAPE {
                request_close();
            }
        }
        CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
    }

    unsafe extern "system" fn fg_proc(
        _hook: HWINEVENTHOOK,
        _event: u32,
        hwnd: HWND,
        _id_obj: i32,
        _id_child: i32,
        _thread: u32,
        _time: u32,
    ) {
        if !ENABLED.load(Ordering::SeqCst) || owns(hwnd) {
            return;
        }
        let gen = SETTLE_GEN.fetch_add(1, Ordering::SeqCst) + 1;
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(SETTLE_MS));
            if SETTLE_GEN.load(Ordering::SeqCst) != gen {
                return;
            }
            if !ENABLED.load(Ordering::SeqCst) {
                return;
            }
            let fg = GetForegroundWindow();
            if owns(fg) {
                return;
            }
            request_close();
        });
    }

    fn pump(app: AppHandle, state: Arc<AppState>) {
        HWND.store(super::popup_hwnd(&app), Ordering::SeqCst);
        let hmod = unsafe { GetModuleHandleW(std::ptr::null()) };
        let mouse = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), hmod, 0) };
        let kb = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_proc), hmod, 0) };
        let fg = unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                std::ptr::null_mut(),
                Some(fg_proc),
                0,
                0,
                WINEVENT_SKIPOWNPROCESS,
            )
        };
        let tid = unsafe { GetCurrentThreadId() };
        PUMP_TID.store(tid, Ordering::SeqCst);
        unsafe {
            let mut msg: MSG = zeroed();
            while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {}
            if !mouse.is_null() {
                UnhookWindowsHookEx(mouse);
            }
            if !kb.is_null() {
                UnhookWindowsHookEx(kb);
            }
            if !fg.is_null() {
                UnhookWinEvent(fg);
            }
        }
        PUMP_TID.store(0, Ordering::SeqCst);
        if ENABLED.load(Ordering::SeqCst) {
            let _ = popup::hide(&app, &state);
        }
    }
}
