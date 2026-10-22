//! Global low-level mouse hook: detects when the user starts/stops dragging
//! a window by tracking left-button-down + rapid mouse motion over a
//! non-client area.
//!
//! Caveat: a low-level mouse hook (`WH_MOUSE_LL`) is injected into every
//! GUI thread. It runs on the installing thread but adds a tiny latency to
//! every mouse event. Keep the callback short.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, SetWindowsHookExW, UnhookWindowsHookEx, HHOOK, MSLLHOOKSTRUCT,
    WH_MOUSE_LL, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
};

/// Shared drag state. The hook callback writes; the main loop reads.
pub struct DragState {
    pub left_down: AtomicBool,
    pub cursor_x: AtomicI32,
    pub cursor_y: AtomicI32,
    pub dragging: AtomicBool,
    /// Pixels of movement after left-down before we call it a "drag".
    pub drag_threshold: i32,
    down_x: AtomicI32,
    down_y: AtomicI32,
}

impl DragState {
    pub const fn new() -> Self {
        Self {
            left_down: AtomicBool::new(false),
            cursor_x: AtomicI32::new(0),
            cursor_y: AtomicI32::new(0),
            dragging: AtomicBool::new(false),
            drag_threshold: 5,
            down_x: AtomicI32::new(0),
            down_y: AtomicI32::new(0),
        }
    }
}

pub static DRAG_STATE: DragState = DragState::new();

static mut HOOK_HANDLE: Option<HHOOK> = None;

pub unsafe fn install() -> windows::core::Result<()> {
    let hook = SetWindowsHookExW(
        WH_MOUSE_LL,
        Some(low_level_mouse_proc),
        HINSTANCE::default(),
        0,
    )?;
    HOOK_HANDLE = Some(hook);
    Ok(())
}

pub unsafe fn uninstall() {
    if let Some(h) = HOOK_HANDLE.take() {
        let _ = UnhookWindowsHookEx(h);
    }
}

unsafe extern "system" fn low_level_mouse_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if code >= 0 {
        let data = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let x = data.pt.x;
        let y = data.pt.y;
        DRAG_STATE.cursor_x.store(x, Ordering::Relaxed);
        DRAG_STATE.cursor_y.store(y, Ordering::Relaxed);

        match wparam.0 as u32 {
            WM_LBUTTONDOWN => {
                DRAG_STATE.left_down.store(true, Ordering::Relaxed);
                DRAG_STATE.down_x.store(x, Ordering::Relaxed);
                DRAG_STATE.down_y.store(y, Ordering::Relaxed);
                DRAG_STATE.dragging.store(false, Ordering::Relaxed);
            }
            WM_LBUTTONUP => {
                DRAG_STATE.left_down.store(false, Ordering::Relaxed);
                DRAG_STATE.dragging.store(false, Ordering::Relaxed);
            }
            WM_MOUSEMOVE => {
                if DRAG_STATE.left_down.load(Ordering::Relaxed)
                    && !DRAG_STATE.dragging.load(Ordering::Relaxed)
                {
                    let dx = (x - DRAG_STATE.down_x.load(Ordering::Relaxed)).abs();
                    let dy = (y - DRAG_STATE.down_y.load(Ordering::Relaxed)).abs();
                    if dx + dy >= DRAG_STATE.drag_threshold {
                        DRAG_STATE.dragging.store(true, Ordering::Relaxed);
                    }
                }
            }
            _ => {}
        }
    }
    unsafe { CallNextHookEx(HHOOK::default(), code, wparam, lparam) }
}
