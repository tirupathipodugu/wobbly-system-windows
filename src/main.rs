//! Wobbly System — Compiz-style wobbly windows for the whole Windows desktop.
//!
//! Architecture:
//!   1. A low-level mouse hook (drag_hook, own thread) swallows left presses
//!      on window captions. A press that moves becomes a wobbly drag; a press
//!      that doesn't is replayed as a normal click.
//!   2. On drag start we mirror the window into a transparent overlay as an
//!      n×n grid of DWM thumbnail tiles, then park the real window just past
//!      the edge of the screen so only the mirror is visible.
//!   3. Each frame: the window's target position follows the cursor, a 4×4
//!      spring model (spring.rs) chases it with the grabbed point pinned,
//!      and the tiles are laid along the Bézier surface over the model.
//!   4. On release the model springs back into shape; once it's at rest the
//!      real window is moved to the final spot and the mirror is removed.
//!
//! Known limits:
//!   - DWM thumbnails stretch but can't warp, so edges bend in small steps
//!     (one per tile) rather than as smooth curves.
//!   - Maximised and elevated (admin) windows keep Windows' normal drag.
//!   - Aero Snap (drag to a screen edge) doesn't trigger on wobbly drags.
//!   - Direct3D exclusive fullscreen apps bypass DWM entirely.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod drag_hook;
mod overlay;
mod spring;
mod thumbnail;

use std::ffi::c_void;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use drag_hook::{DRAG, DRAGGING, IDLE, RELEASED};
use spring::{Model, SpringConfig};
use thumbnail::TiledMirror;
use windows::Win32::Foundation::{HWND, RECT, SIZE};
use windows::Win32::Media::timeBeginPeriod;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetWindowRect, MsgWaitForMultipleObjects, PeekMessageW,
    SetForegroundWindow, SetWindowPos, TranslateMessage, MSG, PM_REMOVE, QS_ALLINPUT,
    SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
};

/// Give up waiting for the wobble to settle after this long.
const MAX_SETTLE: Duration = Duration::from_secs(3);
/// Frame interval while a drag is animating (~144 Hz).
const FRAME_MS: u32 = 7;
/// Time for DWM to show a window move before we cover or uncover it.
const SWAP_DELAY: Duration = Duration::from_millis(40);

fn main() -> windows::core::Result<()> {
    unsafe {
        // Physical pixels everywhere, so hook, window and DWM coordinates agree.
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // 1 ms timer resolution so FRAME_MS waits are accurate.
        timeBeginPeriod(1);
    }
    init_tracing();

    let config = SpringConfig::from_env();
    let tiles = std::env::var("WOBBLY_TILES")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(8)
        .clamp(2, 16);
    tracing::info!(?config, tiles, "wobbly-system starting");

    let overlay = overlay::Overlay::create()?;
    drag_hook::OVERLAY.store(overlay.hwnd.0 as isize, Ordering::Relaxed);
    tracing::info!(x = overlay.x, y = overlay.y, "overlay created");

    drag_hook::spawn();

    let mut active: Option<ActiveDrag> = None;
    let mut last_frame = Instant::now();

    loop {
        // Idle: sleep until the hook pokes the overlay. Active: one pass per
        // FRAME. (Not DwmFlush: it blocks until DWM next composes, which can
        // be never once the wobble has settled and nothing on screen moves.)
        unsafe {
            let timeout = if active.is_some() { FRAME_MS } else { u32::MAX };
            let _ = MsgWaitForMultipleObjects(None, false, timeout, QS_ALLINPUT);
        }
        pump_messages();

        let phase = DRAG.phase.load(Ordering::Acquire);
        if active.is_none() && (phase == DRAGGING || phase == RELEASED) {
            active = ActiveDrag::start(&overlay, config, tiles);
            if active.is_none() {
                DRAG.phase.store(IDLE, Ordering::Release);
            }
            last_frame = Instant::now();
        }

        if let Some(drag) = &mut active {
            let now = Instant::now();
            let dt = (now - last_frame).as_secs_f32();
            last_frame = now;

            if drag.frame(dt, phase == RELEASED, &overlay) {
                active = None;
                overlay.hide();
                DRAG.phase.store(IDLE, Ordering::Release);
            }
        }
    }
}

struct ActiveDrag {
    hwnd: HWND,
    /// Window rect when the drag began.
    rect: RECT,
    down: (i32, i32),
    model: Model,
    /// None when the window couldn't be mirrored: we then just move it.
    mirror: Option<TiledMirror>,
    /// Current top-left the window is heading for.
    origin: [f32; 2],
    released_at: Option<Instant>,
}

impl ActiveDrag {
    fn start(overlay: &overlay::Overlay, config: SpringConfig, tiles: usize) -> Option<Self> {
        let hwnd = HWND(DRAG.target.load(Ordering::Relaxed) as *mut c_void);
        let mut rect = RECT::default();
        if let Err(e) = unsafe { GetWindowRect(hwnd, &mut rect) } {
            tracing::warn!(error = ?e, "target window vanished");
            return None;
        }
        unsafe {
            let _ = SetForegroundWindow(hwnd);
        }

        let down = (
            DRAG.down_x.load(Ordering::Relaxed),
            DRAG.down_y.load(Ordering::Relaxed),
        );
        let model = Model::new(config, rect, down.0, down.1);
        let size = SIZE {
            cx: rect.right - rect.left,
            cy: rect.bottom - rect.top,
        };

        let mirror = match TiledMirror::register(overlay.hwnd, hwnd, tiles, size) {
            Ok(mirror) => Some(mirror),
            Err(e) => {
                tracing::warn!(error = ?e, "could not mirror window; moving it without wobble");
                None
            }
        };

        if let Some(mirror) = &mirror {
            // Show the mirror on top of the real window first, then park the
            // real one off-screen, so there's no frame without either.
            mirror.place(|u, v| model.surface(u, v), (overlay.x, overlay.y));
            overlay.show();
            std::thread::sleep(SWAP_DELAY);
            let (vx, _, vw, _) = overlay::virtual_screen();
            move_window(hwnd, vx + vw + 64, rect.top);
        }

        tracing::info!(?rect, mirrored = mirror.is_some(), "drag started");
        Some(Self {
            hwnd,
            rect,
            down,
            model,
            mirror,
            origin: [rect.left as f32, rect.top as f32],
            released_at: None,
        })
    }

    /// Advance one frame. Returns true when the drag is fully finished.
    fn frame(&mut self, dt: f32, released: bool, overlay: &overlay::Overlay) -> bool {
        if self.released_at.is_none() {
            // The hook freezes the cursor position at release.
            let cx = DRAG.cursor_x.load(Ordering::Relaxed);
            let cy = DRAG.cursor_y.load(Ordering::Relaxed);
            self.origin = [
                (self.rect.left + cx - self.down.0) as f32,
                (self.rect.top + cy - self.down.1) as f32,
            ];
            if released {
                self.released_at = Some(Instant::now());
                self.model.release();
            }
        }
        let (x, y) = (self.origin[0].round() as i32, self.origin[1].round() as i32);

        let Some(mirror) = &self.mirror else {
            move_window(self.hwnd, x, y);
            return self.released_at.is_some();
        };

        self.model.step(dt, self.origin);
        mirror.place(|u, v| self.model.surface(u, v), (overlay.x, overlay.y));

        let Some(released_at) = self.released_at else {
            return false;
        };
        if !self.model.at_rest(self.origin) && released_at.elapsed() < MAX_SETTLE {
            return false;
        }

        // Put the real window back under the mirror, give DWM a moment to
        // show it there, then drop the mirror.
        move_window(self.hwnd, x, y);
        std::thread::sleep(SWAP_DELAY);
        self.mirror = None;
        tracing::info!(x, y, "drag finished");
        true
    }
}

fn move_window(hwnd: HWND, x: i32, y: i32) {
    // Async so a hung app can't stall the render loop.
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            HWND::default(),
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        );
    }
}

fn pump_messages() {
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, HWND::default(), 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            let _ = DispatchMessageW(&msg);
        }
    }
}

/// Logs go to %TEMP%\wobbly-system.log (release builds have no console).
/// Set RUST_LOG=debug for more detail.
fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let path = std::env::temp_dir().join("wobbly-system.log");
    match std::fs::File::create(&path) {
        Ok(file) => {
            let _ = tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_ansi(false)
                .with_writer(std::sync::Mutex::new(file))
                .try_init();
        }
        Err(_) => {
            let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
        }
    }
}
