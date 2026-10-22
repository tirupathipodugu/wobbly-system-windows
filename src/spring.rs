//! Spring-mass sim for window content deformation.
//!
//! A grid of control points covers the window surface. Each point tracks its
//! own displacement from the rest position, with a spring back to that rest
//! position and (optionally) soft constraints to neighbors so the mesh
//! deforms as a connected sheet rather than independent particles.
//!
//! The main loop drives this once per frame with the window's current
//! velocity. Target displacement is proportional to velocity; the spring
//! follows that target with damping — same shape as our Svelte/JS version,
//! promoted to a 2D grid.

use std::time::Instant;

#[derive(Clone, Copy, Debug)]
pub struct SpringConfig {
    pub stiffness: f32,
    pub damping: f32,
    /// Grid resolution. 6×6 is a reasonable floor; 10×10 is smoother.
    pub cols: usize,
    pub rows: usize,
    /// Peak lag distance (pixels) at saturating velocity.
    pub max_lag_px: f32,
    /// Scale factor from pixels/ms velocity to lag pixels.
    pub lag_scale: f32,
    /// Neighbor coupling (0..1). 0 = independent particles, 1 = rigid sheet.
    pub cohesion: f32,
}

impl Default for SpringConfig {
    fn default() -> Self {
        Self {
            stiffness: 0.14,
            damping: 0.78,
            cols: 10,
            rows: 10,
            max_lag_px: 60.0,
            lag_scale: 180.0,
            cohesion: 0.35,
        }
    }
}

/// A single grid point.
#[derive(Clone, Copy, Debug, Default)]
pub struct Point {
    /// Rest position in [0,1]^2 (fraction of window size).
    pub rest_x: f32,
    pub rest_y: f32,
    /// Current displacement (pixels).
    pub dx: f32,
    pub dy: f32,
    /// Current velocity of the displacement spring.
    pub dvx: f32,
    pub dvy: f32,
}

pub struct SpringGrid {
    pub config: SpringConfig,
    pub points: Vec<Point>,
    last_tick: Option<Instant>,
    /// Smoothed window velocity (pixels/ms). Shared across all points — the
    /// per-point target is derived from this plus distance-to-leading-edge.
    pub smoothed_vx: f32,
    pub smoothed_vy: f32,
}

impl SpringGrid {
    pub fn new(config: SpringConfig) -> Self {
        let mut points = Vec::with_capacity(config.cols * config.rows);
        for r in 0..config.rows {
            for c in 0..config.cols {
                points.push(Point {
                    rest_x: (c as f32) / (config.cols as f32 - 1.0),
                    rest_y: (r as f32) / (config.rows as f32 - 1.0),
                    ..Default::default()
                });
            }
        }
        Self {
            config,
            points,
            last_tick: None,
            smoothed_vx: 0.0,
            smoothed_vy: 0.0,
        }
    }

    /// Feed the current window position in pixels. Updates smoothed velocity.
    pub fn feed_position(&mut self, x: i32, y: i32) {
        let now = Instant::now();
        if let Some(prev) = self.last_tick {
            let dt_ms = (now - prev).as_secs_f32() * 1000.0;
            if dt_ms > 0.5 {
                let raw_vx = (x as f32 - self.prev_x) / dt_ms;
                let raw_vy = (y as f32 - self.prev_y) / dt_ms;
                self.smoothed_vx = self.smoothed_vx * 0.6 + raw_vx * 0.4;
                self.smoothed_vy = self.smoothed_vy * 0.6 + raw_vy * 0.4;
            }
        }
        self.prev_x = x as f32;
        self.prev_y = y as f32;
        self.last_tick = Some(now);
    }

    /// Advance the spring sim one step.
    pub fn step(&mut self) {
        let cfg = self.config;
        let target_dx =
            (-self.smoothed_vx * cfg.lag_scale).clamp(-cfg.max_lag_px, cfg.max_lag_px);
        let target_dy =
            (-self.smoothed_vy * cfg.lag_scale).clamp(-cfg.max_lag_px, cfg.max_lag_px);

        // Trailing-edge points lag more. Weight is 0 at leading edge, 1 at trailing.
        let (cols, rows) = (cfg.cols, cfg.rows);
        let leading_x_fac = self.smoothed_vx.signum(); // -1 if moving left, +1 if right
        let leading_y_fac = self.smoothed_vy.signum();

        // Snapshot neighbors to apply cohesion without reading mid-mutation.
        let snapshot: Vec<(f32, f32)> =
            self.points.iter().map(|p| (p.dx, p.dy)).collect();

        for r in 0..rows {
            for c in 0..cols {
                let idx = r * cols + c;
                let p = &mut self.points[idx];

                // Trailing-edge weighting: points away from the leading edge
                // get the full target; points at the leading edge get 0.
                let wx = if leading_x_fac >= 0.0 { 1.0 - p.rest_x } else { p.rest_x };
                let wy = if leading_y_fac >= 0.0 { 1.0 - p.rest_y } else { p.rest_y };
                let point_target_dx = target_dx * (0.3 + 0.7 * wx);
                let point_target_dy = target_dy * (0.3 + 0.7 * wy);

                let acc_x = (point_target_dx - p.dx) * cfg.stiffness;
                let acc_y = (point_target_dy - p.dy) * cfg.stiffness;
                p.dvx = (p.dvx + acc_x) * cfg.damping;
                p.dvy = (p.dvy + acc_y) * cfg.damping;
                p.dx += p.dvx;
                p.dy += p.dvy;

                // Cohesion: pull toward average of 4-neighborhood.
                if cfg.cohesion > 0.0 {
                    let mut sum_x = 0.0;
                    let mut sum_y = 0.0;
                    let mut n = 0.0;
                    for (nr, nc) in &[(r.wrapping_sub(1), c), (r + 1, c), (r, c.wrapping_sub(1)), (r, c + 1)] {
                        if *nr < rows && *nc < cols {
                            let (ndx, ndy) = snapshot[*nr * cols + *nc];
                            sum_x += ndx;
                            sum_y += ndy;
                            n += 1.0;
                        }
                    }
                    if n > 0.0 {
                        let avg_x = sum_x / n;
                        let avg_y = sum_y / n;
                        p.dx = p.dx * (1.0 - cfg.cohesion) + avg_x * cfg.cohesion;
                        p.dy = p.dy * (1.0 - cfg.cohesion) + avg_y * cfg.cohesion;
                    }
                }
            }
        }
    }

    /// True when the mesh is close enough to rest that we can skip rendering.
    pub fn at_rest(&self) -> bool {
        const EPS: f32 = 0.08;
        self.smoothed_vx.abs() < EPS
            && self.smoothed_vy.abs() < EPS
            && self.points.iter().all(|p| {
                p.dx.abs() < EPS && p.dy.abs() < EPS && p.dvx.abs() < EPS && p.dvy.abs() < EPS
            })
    }

    pub fn reset(&mut self) {
        for p in &mut self.points {
            p.dx = 0.0;
            p.dy = 0.0;
            p.dvx = 0.0;
            p.dvy = 0.0;
        }
        self.smoothed_vx = 0.0;
        self.smoothed_vy = 0.0;
    }

    prev_x: f32,
    prev_y: f32,
}

// Simple fields that aren't easy to express in the struct-level block above.
#[cfg(any())] // marker so the compiler doesn't trip
mod _unused {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_has_right_count() {
        let g = SpringGrid::new(SpringConfig::default());
        assert_eq!(g.points.len(), 100);
    }

    #[test]
    fn settles_to_rest_without_motion() {
        let mut g = SpringGrid::new(SpringConfig::default());
        for _ in 0..200 {
            g.step();
        }
        assert!(g.at_rest());
    }
}
