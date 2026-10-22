//! Fullscreen transparent layered overlay window. We draw the deformed copy
//! of the dragged window here while the real one is briefly hidden.
//!
//! The overlay is:
//!   - WS_EX_LAYERED + WS_EX_TRANSPARENT  → click-through + per-pixel alpha
//!   - WS_EX_TOPMOST                       → always above other windows
//!   - WS_POPUP                            → no caption / border
//!   - WS_EX_TOOLWINDOW                    → doesn't appear in Alt-Tab
//!
//! A single overlay covers the primary monitor for v1; multi-monitor is a
//! TODO (needs one overlay per monitor + per-monitor DPI handling).

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, LoadCursorW,
    RegisterClassW, ShowWindow, TranslateMessage, CS_HREDRAW, CS_VREDRAW, HCURSOR, HICON,
    HMENU, IDC_ARROW, MSG, SW_HIDE, SW_SHOW, WM_DESTROY, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

const CLASS_NAME: PCWSTR = w!("WobblySystemOverlay");

pub struct Overlay {
    pub hwnd: HWND,
}

impl Overlay {
    pub fn create(width: i32, height: i32) -> windows::core::Result<Self> {
        unsafe {
            let instance = GetModuleHandleW(None)?.into();
            register_class(instance);

            let hwnd = CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOPMOST
                    | WS_EX_TOOLWINDOW
                    | WS_EX_NOACTIVATE,
                CLASS_NAME,
                w!("WobblySystem"),
                WS_POPUP,
                0,
                0,
                width,
                height,
                HWND::default(),
                HMENU::default(),
                instance,
                None,
            )?;

            let _ = ShowWindow(hwnd, SW_HIDE);
            Ok(Self { hwnd })
        }
    }

    pub fn show(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
        }
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }
}

unsafe fn register_class(instance: HINSTANCE) {
    unsafe {
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wnd_proc),
            hInstance: instance,
            lpszClassName: CLASS_NAME,
            hCursor: LoadCursorW(HINSTANCE::default(), IDC_ARROW)
                .unwrap_or_else(|_| HCURSOR::default()),
            hIcon: HICON::default(),
            hbrBackground: HBRUSH::default(),
            lpszMenuName: PCWSTR::null(),
            cbClsExtra: 0,
            cbWndExtra: 0,
        };
        RegisterClassW(&class);
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_DESTROY => LRESULT(0),
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Pump messages for a single overlay. Call from the main thread.
pub fn pump_once() -> bool {
    unsafe {
        let mut msg = MSG::default();
        let r = GetMessageW(&mut msg, HWND::default(), 0, 0);
        if r.0 <= 0 {
            return false;
        }
        let _ = TranslateMessage(&msg);
        let _ = DispatchMessageW(&msg);
        true
    }
}
