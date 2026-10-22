//! DWM thumbnail management.
//!
//! DWM caches a GPU-side composite of every top-level window. We register a
//! thumbnail that mirrors a source window into our overlay, then control its
//! destination rectangle (position + size) each frame. By offsetting the
//! destination with our spring displacement, the mirrored window appears to
//! deform.
//!
//! LIMITATIONS vs a true shader-deformed render:
//!   - Thumbnails are a per-source single rect. For a true mesh deformation,
//!     we'd need to split each window into tiles and register one thumbnail
//!     per tile, OR capture to our own texture and render through D3D11 with
//!     a vertex shader. v1 ships with a single-rect "whole window lags" look
//!     — proper mesh deformation is a v2 feature.

use windows::Win32::Foundation::{HWND, RECT, SIZE};
use windows::Win32::Graphics::Dwm::{
    DwmQueryThumbnailSourceSize, DwmRegisterThumbnail, DwmUnregisterThumbnail,
    DwmUpdateThumbnailProperties, DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY,
    DWM_TNP_RECTDESTINATION, DWM_TNP_SOURCECLIENTAREAONLY, DWM_TNP_VISIBLE,
    HTHUMBNAIL,
};

pub struct Thumbnail {
    pub handle: HTHUMBNAIL,
    pub source_size: SIZE,
}

impl Thumbnail {
    pub fn register(destination: HWND, source: HWND) -> windows::core::Result<Self> {
        unsafe {
            let handle = DwmRegisterThumbnail(destination, source)?;
            let mut source_size = SIZE::default();
            let _ = DwmQueryThumbnailSourceSize(handle, &mut source_size);
            Ok(Self { handle, source_size })
        }
    }

    /// Place the mirrored window in the overlay with a translated destination
    /// rectangle. Used while dragging to shift the "ghost" relative to the
    /// cursor position.
    pub fn place(&self, dest: RECT, opacity: u8) {
        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTDESTINATION
                | DWM_TNP_OPACITY
                | DWM_TNP_VISIBLE
                | DWM_TNP_SOURCECLIENTAREAONLY,
            rcDestination: dest,
            rcSource: RECT::default(),
            opacity,
            fVisible: true.into(),
            fSourceClientAreaOnly: false.into(),
        };
        unsafe {
            let _ = DwmUpdateThumbnailProperties(self.handle, &props);
        }
    }

    pub fn hide(&self) {
        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_VISIBLE,
            fVisible: false.into(),
            ..Default::default()
        };
        unsafe {
            let _ = DwmUpdateThumbnailProperties(self.handle, &props);
        }
    }
}

impl Drop for Thumbnail {
    fn drop(&mut self) {
        unsafe {
            let _ = DwmUnregisterThumbnail(self.handle);
        }
    }
}
