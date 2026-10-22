//! DWM thumbnail management.
//!
//! DWM keeps a GPU-side image of every top-level window. A thumbnail mirrors
//! a rectangle of that image into our overlay at any destination rectangle.
//! Thumbnails can only be stretched along the axes, not warped, so to bend a
//! window we cut it into an n×n grid of tiles — one thumbnail per tile — and
//! place each tile over the bounding box of its corners on the wobbly
//! surface. Neighbouring boxes overlap by a pixel or two, which hides seams.

use windows::Win32::Foundation::{HWND, RECT, SIZE};
use windows::Win32::Graphics::Dwm::{
    DwmQueryThumbnailSourceSize, DwmRegisterThumbnail, DwmUnregisterThumbnail,
    DwmUpdateThumbnailProperties, DWM_THUMBNAIL_PROPERTIES, DWM_TNP_OPACITY,
    DWM_TNP_RECTDESTINATION, DWM_TNP_RECTSOURCE, DWM_TNP_SOURCECLIENTAREAONLY,
    DWM_TNP_VISIBLE,
};

pub struct Thumbnail {
    /// HTHUMBNAIL; windows 0.58 exposes it as a raw isize.
    handle: isize,
}

impl Thumbnail {
    pub fn register(destination: HWND, source: HWND) -> windows::core::Result<Self> {
        let handle = unsafe { DwmRegisterThumbnail(destination, source)? };
        Ok(Self { handle })
    }

    pub fn source_size(&self) -> SIZE {
        unsafe { DwmQueryThumbnailSourceSize(self.handle).unwrap_or_default() }
    }

    /// Mirror `src` (source-window pixels) to `dest` (overlay pixels).
    pub fn place(&self, src: RECT, dest: RECT) {
        let props = DWM_THUMBNAIL_PROPERTIES {
            dwFlags: DWM_TNP_RECTSOURCE
                | DWM_TNP_RECTDESTINATION
                | DWM_TNP_OPACITY
                | DWM_TNP_VISIBLE
                | DWM_TNP_SOURCECLIENTAREAONLY,
            rcDestination: dest,
            rcSource: src,
            opacity: 255,
            fVisible: true.into(),
            fSourceClientAreaOnly: false.into(),
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

/// A window mirrored as an n×n grid of thumbnail tiles.
pub struct TiledMirror {
    tiles: Vec<Thumbnail>,
    n: usize,
    src: SIZE,
}

impl TiledMirror {
    /// `fallback` is used when DWM can't report the source size.
    pub fn register(
        destination: HWND,
        source: HWND,
        n: usize,
        fallback: SIZE,
    ) -> windows::core::Result<Self> {
        let mut tiles = Vec::with_capacity(n * n);
        for _ in 0..n * n {
            tiles.push(Thumbnail::register(destination, source)?);
        }
        let mut src = tiles[0].source_size();
        if src.cx <= 0 || src.cy <= 0 {
            src = fallback;
        }
        Ok(Self { tiles, n, src })
    }

    /// Lay the tiles along `surface`, which maps (u, v) ∈ [0,1]² to screen
    /// pixels. `origin` is the overlay's top-left in screen pixels.
    pub fn place(&self, surface: impl Fn(f32, f32) -> [f32; 2], origin: (i32, i32)) {
        let n = self.n;
        let stride = n + 1;
        let mut verts = Vec::with_capacity(stride * stride);
        for r in 0..=n {
            for c in 0..=n {
                verts.push(surface(c as f32 / n as f32, r as f32 / n as f32));
            }
        }

        let (w, h) = (self.src.cx, self.src.cy);
        for r in 0..n {
            for c in 0..n {
                let corners = [
                    verts[r * stride + c],
                    verts[r * stride + c + 1],
                    verts[(r + 1) * stride + c],
                    verts[(r + 1) * stride + c + 1],
                ];
                let min_x = corners.iter().map(|p| p[0]).fold(f32::MAX, f32::min);
                let max_x = corners.iter().map(|p| p[0]).fold(f32::MIN, f32::max);
                let min_y = corners.iter().map(|p| p[1]).fold(f32::MAX, f32::min);
                let max_y = corners.iter().map(|p| p[1]).fold(f32::MIN, f32::max);

                let dest = RECT {
                    left: min_x.floor() as i32 - origin.0,
                    top: min_y.floor() as i32 - origin.1,
                    right: max_x.ceil() as i32 - origin.0,
                    bottom: max_y.ceil() as i32 - origin.1,
                };
                let src = RECT {
                    left: w * c as i32 / n as i32,
                    top: h * r as i32 / n as i32,
                    right: w * (c + 1) as i32 / n as i32,
                    bottom: h * (r + 1) as i32 / n as i32,
                };
                self.tiles[r * n + c].place(src, dest);
            }
        }
    }
}
