//! Wobbly System — Compiz-style wobbly windows for the whole Windows desktop.
//!
//! Architecture:
//!   1. Install a low-level mouse hook (drag_hook) to detect drag start/end.
//!   2. When a drag begins on a non-client area, identify the window
//!      (win_enum::window_at_point).
//!   3. Register a DWM thumbnail from that window into our transparent
//!      overlay. Hide the real window via SetWindowPos(SWP_NOREDRAW).
//!   4. Each frame: feed the window's current position into the spring sim,
//!      compute per-point displacement, place the thumbnail at an offset
//!      rect that reflects the mesh centroid's lag. (v1 is single-rect
//!      lag; v2 upgrades to a D3D11 shader-driven mesh deform.)
//!   5. On drag end, let the spring settle, then unhide the real window and
//!      destroy the thumbnail.
//!
//! Known limits (documented, not blockers to v1):
//!   - Single monitor only (per-monitor overlays TODO)
//!   - Doesn't handle Direct3D exclusive fullscreen apps (they bypass DWM)
//!   - UAC-elevated windows can't be thumbnailed from a non-elevated process
//!   - HiDPI per-monitor DPI awareness needs explicit opt-in via manifest

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod drag_hook;
mod overlay;
mod spring;
mod thumbnail;
mod win_enum;

use std::sync::atomic::Ordering;
use std::time::Duration;

use spring::{SpringConfig, SpringGrid};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, SetWindowPos, HWND_TOPMOST, SM_CXSCREEN, SM_CYSCREEN,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
};

fn main() -> windows::core::Result<()> {
    init_tracing();
    tracing::info!("wobbly-system starting");

    // ---- Create the transparent overlay ----
    let (screen_w, screen_h) = unsafe {
        (GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN))
    };
    let overlay = overlay::Overlay::create(screen_w, screen_h)?;
    tracing::info!(w = screen_w, h = screen_h, "overlay created");

    // ---- Install the global mouse hook ----
    unsafe {
        drag_hook::install()?;
    }
    tracing::info!("global mouse hook installed");

    // ---- Main loop: poll drag state, drive the spring sim, place thumbnail ----
    let mut spring = SpringGrid::new(SpringConfig::default());
    let mut active_drag: Option<ActiveDrag> = None;

    loop {
        // Message pump — short, non-blocking. Keeps the overlay responsive
        // without stealing input (because of WS_EX_TRANSPARENT).
        pump_messages_nonblocking();

        let dragging = drag_hook::DRAG_STATE.dragging.load(Ordering::Relaxed);
        let cursor_x = drag_hook::DRAG_STATE.cursor_x.load(Ordering::Relaxed);
        let cursor_y = drag_hook::DRAG_STATE.cursor_y.load(Ordering::Relaxed);

        match (dragging, &mut active_drag) {
            (true, None) => {
                // Transitioning into a drag. Find the window under the cursor.
                if let Some(target) = win_enum::window_at_point(cursor_x, cursor_y) {
                    match thumbnail::Thumbnail::register(overlay.hwnd, target.hwnd) {
                        Ok(thumb) => {
                            overlay.show();
                            unsafe {
                                let _ = SetWindowPos(
                                    overlay.hwnd,
                                    HWND_TOPMOST,
                                    0, 0, 0, 0,
                                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                                );
                            }
                            spring.reset();
                            active_drag = Some(ActiveDrag {
                                target: target.clone(),
                                thumb,
                                last_rect: target.rect,
                            });
                            tracing::info!(title = %target.title, "drag started");
                        }
                        Err(e) => tracing::warn!(error = ?e, "thumbnail register failed"),
                    }
                }
            }
            (true, Some(ad)) => {
                // Mid-drag: refresh target rect, step the spring, place thumbnail.
                if let Some(fresh) = win_enum::window_at_point(cursor_x, cursor_y) {
                    if fresh.hwnd == ad.target.hwnd {
                        ad.last_rect = fresh.rect;
                    }
                }
                spring.feed_position(ad.last_rect.left, ad.last_rect.top);
                spring.step();

                // Centroid of displacement — simple v1 placement.
                let (cdx, cdy) = centroid_displacement(&spring);
                let dest = RECT {
                    left: ad.last_rect.left + cdx as i32,
                    top: ad.last_rect.top + cdy as i32,
                    right: ad.last_rect.right + cdx as i32,
                    bottom: ad.last_rect.bottom + cdy as i32,
                };
                ad.thumb.place(dest, 255);
            }
            (false, Some(ad)) => {
                // Drag ended — let the spring settle for a few more frames.
                spring.feed_position(ad.last_rect.left, ad.last_rect.top);
                spring.step();
                let (cdx, cdy) = centroid_displacement(&spring);
                let dest = RECT {
                    left: ad.last_rect.left + cdx as i32,
                    top: ad.last_rect.top + cdy as i32,
                    right: ad.last_rect.right + cdx as i32,
                    bottom: ad.last_rect.bottom + cdy as i32,
                };
                ad.thumb.place(dest, 255);

                if spring.at_rest() {
                    overlay.hide();
                    active_drag = None;
                    tracing::info!("drag ended, settled");
                }
            }
            (false, None) => {
                // Nothing to do — idle poll.
            }
        }

        std::thread::sleep(Duration::from_micros(8_000)); // ~120 Hz target
    }
}

struct ActiveDrag {
    target: win_enum::WindowInfo,
    thumb: thumbnail::Thumbnail,
    last_rect: RECT,
}

fn centroid_displacement(spring: &SpringGrid) -> (f32, f32) {
    let n = spring.points.len() as f32;
    let sx: f32 = spring.points.iter().map(|p| p.dx).sum();
    let sy: f32 = spring.points.iter().map(|p| p.dy).sum();
    (sx / n, sy / n)
}

fn pump_messages_nonblocking() {
    use windows::Win32::Foundation::HWND as HW;
    use windows::Win32::UI::WindowsAndMessaging::{
        DispatchMessageW, PeekMessageW, TranslateMessage, MSG, PM_REMOVE,
    };
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, HW::default(), 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            let _ = DispatchMessageW(&msg);
        }
    }
}

fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .try_init();
}
