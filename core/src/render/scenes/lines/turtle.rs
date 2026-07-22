//! Turtle interpretation: walk an L-system string into line segments, with a
//! branch stack for `[`/`]`. A build-time step (runs inside `Scene::configure`,
//! off the hot path) that produces the base geometry a generator scene caches
//! and then only transforms per frame.
//!
//! Commands (the common turtle vocabulary):
//! - `F`, `G` — step forward, drawing a segment
//! - `f`      — step forward without drawing
//! - `+`      — turn left by the configured angle
//! - `-`      — turn right by the configured angle
//! - `[`      — push position + heading
//! - `]`      — pop position + heading
//! - anything else — no-op (grammar variables such as `X` that only expand)

// Under render/, so it carries the panic pragma even though it runs only at
// preset load. Written panic-free (no unwrap/index/panic).
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

use std::f32::consts::FRAC_PI_2;

use super::renderer::SegmentInstance;

/// Walk `s` into `out` (cleared first) as base geometry — positions only; the
/// scene fills colour/width per frame. `angle` is in radians. Segments beyond
/// `max_segments` are dropped and counted (the ADR-0007 cap is never silent):
/// the returned `usize` is how many draw steps were dropped.
pub fn walk(s: &str, angle: f32, max_segments: usize, out: &mut Vec<SegmentInstance>) -> usize {
    out.clear();

    // Start at the origin pointing up; the whole figure is fit-normalized after.
    let step = 1.0_f32;
    let mut x = 0.0_f32;
    let mut y = 0.0_f32;
    let mut heading = FRAC_PI_2;
    let mut stack: Vec<(f32, f32, f32)> = Vec::new();
    let mut dropped = 0usize;

    for ch in s.chars() {
        match ch {
            'F' | 'G' => {
                let (dy, dx) = heading.sin_cos();
                let nx = x + dx * step;
                let ny = y + dy * step;
                if out.len() < max_segments {
                    out.push(SegmentInstance {
                        a: [x, y],
                        b: [nx, ny],
                        color: [1.0, 1.0, 1.0],
                        width: 0.01,
                    });
                } else {
                    dropped += 1;
                }
                x = nx;
                y = ny;
            }
            'f' => {
                let (dy, dx) = heading.sin_cos();
                x += dx * step;
                y += dy * step;
            }
            '+' => heading += angle,
            '-' => heading -= angle,
            '[' => stack.push((x, y, heading)),
            ']' => {
                if let Some((px, py, ph)) = stack.pop() {
                    x = px;
                    y = py;
                    heading = ph;
                }
            }
            _ => {}
        }
    }
    dropped
}

/// Center `segs` and uniformly scale them to fit within `[-target, target]` on
/// the larger axis, so any figure (whatever its raw extent per depth) frames
/// itself in the view. A degenerate (zero-extent) set is left untouched.
pub fn normalize_fit(segs: &mut [SegmentInstance], target: f32) {
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for seg in segs.iter() {
        for p in [seg.a, seg.b] {
            min_x = min_x.min(p[0]);
            min_y = min_y.min(p[1]);
            max_x = max_x.max(p[0]);
            max_y = max_y.max(p[1]);
        }
    }
    let extent = (max_x - min_x).max(max_y - min_y);
    if !extent.is_finite() || extent <= f32::EPSILON {
        return;
    }
    let cx = 0.5 * (min_x + max_x);
    let cy = 0.5 * (min_y + max_y);
    let scale = 2.0 * target / extent;
    for seg in segs.iter_mut() {
        seg.a = [(seg.a[0] - cx) * scale, (seg.a[1] - cy) * scale];
        seg.b = [(seg.b[0] - cx) * scale, (seg.b[1] - cy) * scale];
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn walk_produces_one_segment_per_draw_step() {
        let mut out = Vec::with_capacity(16);
        // A closed square: four forward steps turning 90 degrees.
        walk("F+F+F+F", std::f32::consts::FRAC_PI_2, 100, &mut out);
        assert_eq!(out.len(), 4, "four F steps -> four segments");

        // A branch: the bracketed F is a third segment; `]` restores state so
        // the trailing F continues from the branch point.
        out.clear();
        walk("F[+F]F", std::f32::consts::FRAC_PI_2, 100, &mut out);
        assert_eq!(out.len(), 3, "trunk + branch + trunk");
    }

    #[test]
    fn walk_is_deterministic_for_a_fixed_structure() {
        let mut a = Vec::with_capacity(64);
        let mut b = Vec::with_capacity(64);
        let s = "FF+F[-F]+FF";
        walk(s, 0.4, 100, &mut a);
        walk(s, 0.4, 100, &mut b);
        assert_eq!(a, b, "same string + angle -> identical geometry");
    }

    #[test]
    fn the_segment_cap_truncates_and_reports_the_drop() {
        let mut out = Vec::with_capacity(8);
        // Ten draw steps, but a cap of 3: seven are dropped and counted.
        let dropped = walk("FFFFFFFFFF", 0.0, 3, &mut out);
        assert_eq!(out.len(), 3, "only the cap is kept");
        assert_eq!(dropped, 7, "the overflow is counted, never silent");
    }

    #[test]
    fn normalize_fit_centers_and_scales_into_the_target_box() {
        let mut out = Vec::with_capacity(16);
        walk("F+F+F+F", std::f32::consts::FRAC_PI_2, 100, &mut out);
        normalize_fit(&mut out, 0.9);
        for seg in &out {
            for p in [seg.a, seg.b] {
                assert!(p[0].abs() <= 0.9 + 1e-4 && p[1].abs() <= 0.9 + 1e-4);
            }
        }
    }
}
