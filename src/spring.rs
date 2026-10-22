//! Compiz-style wobbly model.
//!
//! A 4×4 grid of masses covers the window. Each mass is tied to its four
//! neighbours by springs that pull toward their rest *offset* (vector
//! springs), so the sheet resists stretching and shearing alike. While
//! dragging, the mass nearest the grab point is pinned to the window and the
//! rest of the sheet trails behind it. After release every mass also gets a
//! spring back to its rest spot, so the sheet jiggles and settles onto the
//! window's final position. The drawn surface is a bicubic Bézier patch over
//! the 16 masses — the same construction Compiz uses.

use windows::Win32::Foundation::RECT;

/// Masses per side.
pub const GRID: usize = 4;
const N: usize = GRID * GRID;

#[derive(Clone, Copy, Debug)]
pub struct SpringConfig {
    /// Neighbour spring constant (1/s²). Higher = stiffer, less lag.
    pub stiffness: f32,
    /// Velocity damping (1/s). Lower = more jiggle after release.
    pub friction: f32,
    /// Pull back to the rest shape once the window is let go (1/s²).
    pub settle_stiffness: f32,
}

impl Default for SpringConfig {
    fn default() -> Self {
        Self {
            stiffness: 2500.0,
            friction: 14.0,
            settle_stiffness: 600.0,
        }
    }
}

impl SpringConfig {
    /// Defaults, overridable with WOBBLY_SPRING / WOBBLY_FRICTION /
    /// WOBBLY_SETTLE so the feel can be tuned without a rebuild.
    pub fn from_env() -> Self {
        let read = |name: &str, default: f32| {
            std::env::var(name)
                .ok()
                .and_then(|v| v.parse::<f32>().ok())
                .filter(|v| v.is_finite() && *v > 0.0)
                .unwrap_or(default)
        };
        let d = Self::default();
        Self {
            stiffness: read("WOBBLY_SPRING", d.stiffness),
            friction: read("WOBBLY_FRICTION", d.friction),
            settle_stiffness: read("WOBBLY_SETTLE", d.settle_stiffness),
        }
    }
}

pub struct Model {
    config: SpringConfig,
    /// Rest position of each mass relative to the window's top-left.
    rest: [[f32; 2]; N],
    /// Current position of each mass in screen pixels.
    pos: [[f32; 2]; N],
    vel: [[f32; 2]; N],
    /// Mass pinned to the window while dragging; None once released.
    grabbed: Option<usize>,
}

impl Model {
    pub fn new(config: SpringConfig, rect: RECT, grab_x: i32, grab_y: i32) -> Self {
        let w = (rect.right - rect.left) as f32;
        let h = (rect.bottom - rect.top) as f32;
        let mut rest = [[0.0; 2]; N];
        let mut pos = [[0.0; 2]; N];
        for r in 0..GRID {
            for c in 0..GRID {
                let i = r * GRID + c;
                rest[i] = [
                    w * c as f32 / (GRID - 1) as f32,
                    h * r as f32 / (GRID - 1) as f32,
                ];
                pos[i] = [rect.left as f32 + rest[i][0], rect.top as f32 + rest[i][1]];
            }
        }

        let dist2 = |p: &[f32; 2]| {
            let dx = p[0] - grab_x as f32;
            let dy = p[1] - grab_y as f32;
            dx * dx + dy * dy
        };
        let grabbed = (0..N).min_by(|&a, &b| dist2(&pos[a]).total_cmp(&dist2(&pos[b])));

        Self {
            config,
            rest,
            pos,
            vel: [[0.0; 2]; N],
            grabbed,
        }
    }

    pub fn release(&mut self) {
        self.grabbed = None;
    }

    /// Advance the sim by `dt` seconds with the window's top-left at `origin`.
    pub fn step(&mut self, dt: f32, origin: [f32; 2]) {
        // Fixed small substeps keep the explicit integrator stable at any
        // frame rate.
        const MAX_SUBSTEP: f32 = 1.0 / 480.0;
        let dt = dt.clamp(0.0, 0.05);
        let steps = (dt / MAX_SUBSTEP).ceil().max(1.0) as usize;
        let h = dt / steps as f32;
        for _ in 0..steps {
            self.substep(h, origin);
        }
    }

    fn substep(&mut self, h: f32, origin: [f32; 2]) {
        let cfg = self.config;
        let mut acc = [[0.0f32; 2]; N];
        for r in 0..GRID {
            for c in 0..GRID {
                let i = r * GRID + c;
                for (dr, dc) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (nr, nc) = (r as i32 + dr, c as i32 + dc);
                    if nr < 0 || nc < 0 || nr >= GRID as i32 || nc >= GRID as i32 {
                        continue;
                    }
                    let j = nr as usize * GRID + nc as usize;
                    for a in 0..2 {
                        let stretch =
                            (self.pos[j][a] - self.pos[i][a]) - (self.rest[j][a] - self.rest[i][a]);
                        acc[i][a] += cfg.stiffness * stretch;
                    }
                }
                for a in 0..2 {
                    if self.grabbed.is_none() {
                        let target = origin[a] + self.rest[i][a];
                        acc[i][a] += cfg.settle_stiffness * (target - self.pos[i][a]);
                    }
                    acc[i][a] -= cfg.friction * self.vel[i][a];
                }
            }
        }

        for i in 0..N {
            if Some(i) == self.grabbed {
                self.pos[i] = [origin[0] + self.rest[i][0], origin[1] + self.rest[i][1]];
                self.vel[i] = [0.0; 2];
                continue;
            }
            for a in 0..2 {
                self.vel[i][a] += acc[i][a] * h;
                self.pos[i][a] += self.vel[i][a] * h;
            }
        }
    }

    /// True once every mass sits (and stays) on its rest spot.
    pub fn at_rest(&self, origin: [f32; 2]) -> bool {
        (0..N).all(|i| {
            (0..2).all(|a| {
                (self.pos[i][a] - origin[a] - self.rest[i][a]).abs() < 0.5
                    && self.vel[i][a].abs() < 8.0
            })
        })
    }

    /// Screen position of the Bézier surface at (u, v) ∈ [0,1]².
    pub fn surface(&self, u: f32, v: f32) -> [f32; 2] {
        let bu = bernstein(u);
        let bv = bernstein(v);
        let mut out = [0.0; 2];
        for r in 0..GRID {
            for c in 0..GRID {
                let w = bv[r] * bu[c];
                let p = self.pos[r * GRID + c];
                out[0] += w * p[0];
                out[1] += w * p[1];
            }
        }
        out
    }
}

fn bernstein(t: f32) -> [f32; GRID] {
    let s = 1.0 - t;
    [s * s * s, 3.0 * t * s * s, 3.0 * t * t * s, t * t * t]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> RECT {
        RECT { left: 100, top: 50, right: 900, bottom: 650 }
    }

    #[test]
    fn surface_matches_window_at_rest() {
        let m = Model::new(SpringConfig::default(), rect(), 500, 60);
        let p = m.surface(0.25, 0.5);
        assert!((p[0] - 300.0).abs() < 0.01 && (p[1] - 350.0).abs() < 0.01);
    }

    #[test]
    fn far_corner_lags_while_dragging() {
        let mut m = Model::new(SpringConfig::default(), rect(), 100, 50);
        // Drag right at 2000 px/s for a tenth of a second.
        for k in 1..=6 {
            m.step(1.0 / 60.0, [100.0 + 2000.0 * k as f32 / 60.0, 50.0]);
        }
        let origin_x = 100.0 + 200.0;
        let far = m.surface(1.0, 1.0);
        assert!(far[0] < origin_x + 800.0 - 10.0, "far corner should trail: {far:?}");
    }

    #[test]
    fn settles_after_release() {
        let mut m = Model::new(SpringConfig::default(), rect(), 100, 50);
        for k in 1..=6 {
            m.step(1.0 / 60.0, [100.0 + 2000.0 * k as f32 / 60.0, 50.0]);
        }
        m.release();
        let origin = [300.0, 50.0];
        for _ in 0..600 {
            m.step(1.0 / 60.0, origin);
        }
        assert!(m.at_rest(origin));
    }
}
