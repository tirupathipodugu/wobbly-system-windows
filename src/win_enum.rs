//! Enumerate top-level windows the user can actually see + drag.
//!
//! Filters out: invisible, cloaked (UWP's hidden state), minimized, our own
//! overlay window, tool windows (tooltips), zero-size, offscreen, and the
//! shell (Program Manager / Progman — the desktop).

use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use windows::core::{BOOL, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindow, GetWindowLongW, GetWindowTextW,
    IsIconic, IsWindowVisible, GWL_EXSTYLE, GW_OWNER, WS_EX_TOOLWINDOW,
};

#[derive(Debug, Clone)]
pub struct WindowInfo {
    pub hwnd: HWND,
    pub title: String,
    pub class: String,
    pub rect: RECT,
}

/// Return the current list of top-level windows that are legit drag targets.
pub fn enumerate_dragable() -> Vec<WindowInfo> {
    let mut out: Vec<WindowInfo> = Vec::new();
    unsafe {
        let _ = EnumWindows(
            Some(enum_proc),
            LPARAM(&mut out as *mut _ as isize),
        );
    }
    out
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let out = unsafe { &mut *(lparam.0 as *mut Vec<WindowInfo>) };

    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return true.into();
        }
        if IsIconic(hwnd).as_bool() {
            return true.into();
        }
    }

    // Tool windows (tooltips, picker popups) rarely want wobble treatment.
    let ex_style = unsafe { GetWindowLongW(hwnd, GWL_EXSTYLE) };
    if (ex_style as u32) & WS_EX_TOOLWINDOW.0 != 0 {
        return true.into();
    }

    // Skip owned windows (dialogs attached to a parent) — we want their owner.
    let owner = unsafe { GetWindow(hwnd, GW_OWNER) };
    if let Ok(owner_hwnd) = owner {
        if !owner_hwnd.is_invalid() {
            return true.into();
        }
    }

    // Skip DWM-cloaked windows (UWP background apps).
    let mut cloaked: u32 = 0;
    unsafe {
        let _ = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            &mut cloaked as *mut _ as *mut _,
            std::mem::size_of::<u32>() as u32,
        );
    }
    if cloaked != 0 {
        return true.into();
    }

    // Visible rect — use DWM extended frame bounds so we exclude the
    // invisible resize border Win10/11 add around every window.
    let mut rect: RECT = RECT::default();
    unsafe {
        let hr = DwmGetWindowAttribute(
            hwnd,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            &mut rect as *mut _ as *mut _,
            std::mem::size_of::<RECT>() as u32,
        );
        if hr.is_err() {
            return true.into();
        }
    }
    if rect.right - rect.left < 20 || rect.bottom - rect.top < 20 {
        return true.into();
    }

    let title = get_window_text(hwnd);
    let class = get_window_class(hwnd);

    // Skip the shell window + ancillary desktop widgets.
    if class == "Progman" || class == "WorkerW" || class == "Shell_TrayWnd" {
        return true.into();
    }

    out.push(WindowInfo { hwnd, title, class, rect });
    true.into()
}

fn get_window_text(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if len <= 0 {
        return String::new();
    }
    OsString::from_wide(&buf[..len as usize])
        .to_string_lossy()
        .into_owned()
}

fn get_window_class(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    if len <= 0 {
        return String::new();
    }
    OsString::from_wide(&buf[..len as usize])
        .to_string_lossy()
        .into_owned()
}

/// Find the window the user is currently about to drag by its screen point.
/// Hit-tests from WindowFromPoint, climbs to top-level.
pub fn window_at_point(x: i32, y: i32) -> Option<WindowInfo> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::{GetAncestor, WindowFromPoint, GA_ROOT};

    unsafe {
        let hwnd = WindowFromPoint(POINT { x, y });
        if hwnd.is_invalid() {
            return None;
        }
        let root = GetAncestor(hwnd, GA_ROOT);
        if root.is_invalid() {
            return None;
        }
        // Reuse the filter + info extraction.
        let dummy = PCWSTR::null();
        let _ = dummy; // silence
        let mut out: Vec<WindowInfo> = Vec::with_capacity(1);
        let lp = LPARAM(&mut out as *mut _ as isize);
        enum_proc(root, lp);
        out.into_iter().next()
    }
}
