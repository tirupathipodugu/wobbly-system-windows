//! Transparent, click-through overlay covering every monitor. The bent copy
//! of the dragged window is drawn here (as DWM thumbnails) while the real
//! window is parked off-screen.
//!
//! The overlay is:
//!   - WS_EX_LAYERED + WS_EX_TRANSPARENT  → click-through
//!   - DWM frame extended over the client → black paints as fully transparent
//!   - WS_EX_TOPMOST                       → above other windows
//!   - WS_POPUP                            → no caption / border
//!   - WS_EX_TOOLWINDOW                    → doesn't appear in Alt-Tab

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
use windows::Win32::Graphics::Gdi::{GetStockObject, BLACK_BRUSH, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::MARGINS;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetSystemMetrics, LoadCursorW, RegisterClassW,
    SetLayeredWindowAttributes, SetWindowPos, ShowWindow, CS_HREDRAW, CS_VREDRAW, HCURSOR,
    HICON, HMENU, HWND_TOPMOST, IDC_ARROW, LWA_ALPHA, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE,
    SW_SHOWNOACTIVATE, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

const CLASS_NAME: PCWSTR = w!("WobblySystemOverlay");

pub struct Overlay {
    pub hwnd: HWND,
    /// Top-left of the virtual screen, in screen pixels.
    pub x: i32,
    pub y: i32,
}

impl Overlay {
    pub fn create() -> windows::core::Result<Self> {
        unsafe {
            let (x, y, width, height) = virtual_screen();
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
                x,
                y,
                width,
                height,
                HWND::default(),
                HMENU::default(),
                instance,
                None,
            )?;

            SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA)?;
            let glass = MARGINS {
                cxLeftWidth: -1,
                cxRightWidth: -1,
                cyTopHeight: -1,
                cyBottomHeight: -1,
            };
            DwmExtendFrameIntoClientArea(hwnd, &glass)?;

            let _ = ShowWindow(hwnd, SW_HIDE);
            Ok(Self { hwnd, x, y })
        }
    }

    pub fn show(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            let _ = SetWindowPos(
                self.hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
    }

    pub fn hide(&self) {
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
    }
}

/// (x, y, width, height) of the box spanning every monitor.
pub fn virtual_screen() -> (i32, i32, i32, i32) {
    unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        )
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
            // Black under an extended DWM frame renders fully transparent.
            hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
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
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}
