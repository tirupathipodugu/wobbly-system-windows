//! Global low-level mouse hook that takes over title-bar drags.
//!
//! A left press on a window's caption is swallowed so Windows never starts
//! its own move loop. If the mouse then travels past a small threshold the
//! press becomes a wobbly drag, which the render loop picks up from
//! `DRAG.phase`. If the button comes back up first, it was just a click: we
//! replay it with SendInput (tagged so the hook lets it through), which keeps
//! title-bar clicks and double-click-to-maximise working.
//!
//! The hook runs on its own thread with its own message loop, so input is
//! never held up by the render loop.

use std::ffi::c_void;
use std::sync::atomic::{AtomicI32, AtomicIsize, AtomicU8, Ordering};
use std::sync::OnceLock;

use windows::Win32::Foundation::{
    CloseHandle, HANDLE, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM,
};
use windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentThreadId, OpenProcess, OpenProcessToken,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetAncestor, GetClassNameW, GetMessageW, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible, IsZoomed, PostMessageW, PostThreadMessageW, SendMessageTimeoutW,
    SetWindowsHookExW, UnhookWindowsHookEx, WindowFromPoint, GA_ROOT, HHOOK, HTCAPTION,
    HTTRANSPARENT, LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, SMTO_ABORTIFHUNG, SMTO_BLOCK, WH_MOUSE_LL,
    WM_APP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCHITTEST,
};

/// No drag in progress; the next caption press may be taken over.
pub const IDLE: u8 = 0;
/// Caption press swallowed; waiting to see if it becomes a drag.
pub const PRESSED: u8 = 1;
/// Button down and moving: the render loop owns the window.
pub const DRAGGING: u8 = 2;
/// Button released; the render loop lets the wobble settle, then sets IDLE.
pub const RELEASED: u8 = 3;

/// Shared drag state. The hook writes; the render loop reads.
pub struct DragState {
    pub phase: AtomicU8,
    /// Top-level window being dragged, as a raw HWND.
    pub target: AtomicIsize,
    pub down_x: AtomicI32,
    pub down_y: AtomicI32,
    /// Latest cursor position; frozen once the button is released.
    pub cursor_x: AtomicI32,
    pub cursor_y: AtomicI32,
}

pub static DRAG: DragState = DragState {
    phase: AtomicU8::new(IDLE),
    target: AtomicIsize::new(0),
    down_x: AtomicI32::new(0),
    down_y: AtomicI32::new(0),
    cursor_x: AtomicI32::new(0),
    cursor_y: AtomicI32::new(0),
};

/// The overlay window: never a drag target, and poked to wake the render
/// loop when a drag starts or ends.
pub static OVERLAY: AtomicIsize = AtomicIsize::new(0);

/// Pixels of travel after a caption press before it counts as a drag.
const DRAG_THRESHOLD: i32 = 4;
/// dwExtraInfo tag on replayed clicks so the hook passes them through.
const REPLAY_MARKER: usize = 0x5767_6C79;
const WM_REPLAY_CLICK: u32 = WM_APP + 1;
/// Desktop and taskbar windows, which must keep their own drag behaviour.
const SHELL_CLASSES: &[&str] = &[
    "Progman",
    "WorkerW",
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
];

/// Start the hook thread.
pub fn spawn() {
    let spawned = std::thread::Builder::new()
        .name("mouse-hook".into())
        .spawn(|| unsafe {
            let instance: HINSTANCE = match GetModuleHandleW(None) {
                Ok(m) => m.into(),
                Err(e) => {
                    tracing::error!(error = ?e, "GetModuleHandleW failed");
                    return;
                }
            };
            let hook = match SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), instance, 0) {
                Ok(h) => h,
                Err(e) => {
                    tracing::error!(error = ?e, "mouse hook install failed");
                    return;
                }
            };
            tracing::info!("global mouse hook installed");

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, HWND::default(), 0, 0).0 > 0 {
                if msg.message == WM_REPLAY_CLICK {
                    replay_click();
                }
            }
            let _ = UnhookWindowsHookEx(hook);
        });
    if let Err(e) = spawned {
        tracing::error!(error = ?e, "could not start hook thread");
    }
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 && handle(wparam.0 as u32, unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) }) {
        // Swallow the event.
        return LRESULT(1);
    }
    unsafe { CallNextHookEx(HHOOK::default(), code, wparam, lparam) }
}

/// Returns true to swallow the event.
fn handle(msg: u32, data: &MSLLHOOKSTRUCT) -> bool {
    if data.flags & LLMHF_INJECTED != 0 && data.dwExtraInfo == REPLAY_MARKER {
        return false;
    }
    let (x, y) = (data.pt.x, data.pt.y);
    let phase = DRAG.phase.load(Ordering::Acquire);
    if phase != RELEASED {
        DRAG.cursor_x.store(x, Ordering::Relaxed);
        DRAG.cursor_y.store(y, Ordering::Relaxed);
    }

    match (msg, phase) {
        (WM_LBUTTONDOWN, IDLE) => {
            let Some(target) = caption_window_at(x, y) else {
                return false;
            };
            DRAG.target.store(target.0 as isize, Ordering::Relaxed);
            DRAG.down_x.store(x, Ordering::Relaxed);
            DRAG.down_y.store(y, Ordering::Relaxed);
            DRAG.phase.store(PRESSED, Ordering::Release);
            true
        }
        (WM_MOUSEMOVE, PRESSED) => {
            let dx = (x - DRAG.down_x.load(Ordering::Relaxed)).abs();
            let dy = (y - DRAG.down_y.load(Ordering::Relaxed)).abs();
            if dx + dy >= DRAG_THRESHOLD {
                DRAG.phase.store(DRAGGING, Ordering::Release);
                wake_render_loop();
            }
            false
        }
        (WM_LBUTTONUP, PRESSED) => {
            // Just a click: hand it back to the window.
            DRAG.phase.store(IDLE, Ordering::Release);
            unsafe {
                let _ = PostThreadMessageW(
                    GetCurrentThreadId(),
                    WM_REPLAY_CLICK,
                    WPARAM(0),
                    LPARAM(0),
                );
            }
            true
        }
        (WM_LBUTTONUP, DRAGGING) => {
            DRAG.phase.store(RELEASED, Ordering::Release);
            wake_render_loop();
            true
        }
        _ => false,
    }
}

fn wake_render_loop() {
    let overlay = HWND(OVERLAY.load(Ordering::Relaxed) as *mut c_void);
    unsafe {
        let _ = PostMessageW(overlay, WM_APP, WPARAM(0), LPARAM(0));
    }
}

fn replay_click() {
    let click = |flags| INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dwFlags: flags,
                dwExtraInfo: REPLAY_MARKER,
                ..Default::default()
            },
        },
    };
    let inputs = [click(MOUSEEVENTF_LEFTDOWN), click(MOUSEEVENTF_LEFTUP)];
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// The top-level window whose caption is under (x, y), if it's one we can
/// take over.
fn caption_window_at(x: i32, y: i32) -> Option<HWND> {
    unsafe {
        let child = WindowFromPoint(POINT { x, y });
        if child.is_invalid() {
            return None;
        }
        let root = GetAncestor(child, GA_ROOT);
        if root.is_invalid() || root.0 as isize == OVERLAY.load(Ordering::Relaxed) {
            return None;
        }
        // Maximised windows keep Windows' own drag (it restores them first).
        if !IsWindowVisible(root).as_bool()
            || IsIconic(root).as_bool()
            || IsZoomed(root).as_bool()
        {
            return None;
        }
        if SHELL_CLASSES.contains(&window_class(root).as_str()) {
            return None;
        }
        // We can't move windows of elevated (admin) apps, so leave them be.
        if !same_or_lower_integrity(root) {
            return None;
        }

        let mut hit = hit_test(child, x, y)?;
        if hit == HTTRANSPARENT as isize && child != root {
            hit = hit_test(root, x, y)?;
        }
        (hit == HTCAPTION as isize).then_some(root)
    }
}

fn hit_test(hwnd: HWND, x: i32, y: i32) -> Option<isize> {
    let lparam = ((y as u16 as u32) << 16 | x as u16 as u32) as i32 as isize;
    let mut result = 0usize;
    let ok = unsafe {
        SendMessageTimeoutW(
            hwnd,
            WM_NCHITTEST,
            WPARAM(0),
            LPARAM(lparam),
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            50,
            Some(&mut result as *mut usize),
        )
    };
    (ok.0 != 0).then_some(result as isize)
}

fn window_class(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..len.max(0) as usize])
}

fn same_or_lower_integrity(hwnd: HWND) -> bool {
    static OWN: OnceLock<Option<u32>> = OnceLock::new();
    let own = *OWN.get_or_init(|| unsafe { token_integrity(GetCurrentProcess()) });

    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid as *mut u32)) };
    if pid == 0 {
        return false;
    }
    let theirs = unsafe {
        match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
            Ok(process) => {
                let level = token_integrity(process);
                let _ = CloseHandle(process);
                level
            }
            Err(_) => None,
        }
    };
    matches!((theirs, own), (Some(t), Some(o)) if t <= o)
}

/// Mandatory integrity level (the last sub-authority of the label SID).
unsafe fn token_integrity(process: HANDLE) -> Option<u32> {
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
        // u64 storage keeps the TOKEN_MANDATORY_LABEL pointer-aligned.
        let mut buf = [0u64; 16];
        let mut len = 0u32;
        let got = GetTokenInformation(
            token,
            TokenIntegrityLevel,
            Some(buf.as_mut_ptr() as *mut c_void),
            std::mem::size_of_val(&buf) as u32,
            &mut len,
        );
        let _ = CloseHandle(token);
        got.ok()?;

        let label = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
        let sid = label.Label.Sid;
        let count = *GetSidSubAuthorityCount(sid);
        if count == 0 {
            return None;
        }
        Some(*GetSidSubAuthority(sid, count as u32 - 1))
    }
}
