//! Parametric curve samplers: pure `t -> (x, y)` functions written straight
//! into a preallocated segment buffer. Cheap enough to resample every frame
//! (ADR-0007 parametric build model), so continuous audio can sweep the shape
//! live. Deterministic: no wall-clock, no randomness — the same parameters
//! always yield the same segments (NFR 6).

// Hot-path panic-denial pragma: the sampler runs every displayed frame.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

use super::renderer::SegmentInstance;

/// Sample a Maurer rose into `out` (cleared first), applying `scale`,
/// `rotation` (radians), a single `color`, and per-segment `width`.
///
/// A Maurer rose walks `samples` points at a fixed angular step `d` degrees,
/// with radius `sin(n * theta)`; connecting the successive chords is what draws
/// the characteristic web. `draw_progress` in `0..=1` reveals the curve from the
/// start (line-draw-on); `1.0` draws it whole.
///
/// Allocation-free: the caller preallocates `out` with capacity `>= samples`,
/// and this pushes at most `samples` segments (never exceeding that capacity),
/// so no reallocation occurs on the hot path.
#[allow(clippy::too_many_arguments)]
pub fn maurer_rose(
    n: f32,
    d: f32,
    samples: usize,
    scale: f32,
    rotation: f32,
    draw_progress: f32,
    color: [f32; 3],
    width: f32,
    out: &mut Vec<SegmentInstance>,
) {
    out.clear();
    if samples == 0 {
        return;
    }

    let (rot_sin, rot_cos) = rotation.sin_cos();
    // How many of the `samples` chords to draw (line-draw-on).
    let progress = draw_progress.clamp(0.0, 1.0);
    let drawn = ((samples as f32) * progress).round() as usize;
    let drawn = drawn.min(samples);

    let point = |k: usize| -> [f32; 2] {
        let theta = (k as f32 * d).to_radians();
        let r = (n * theta).sin();
        let (ts, tc) = theta.sin_cos();
        // Base point in the unit disc, then rotate and scale.
        let x = r * tc;
        let y = r * ts;
        [
            (x * rot_cos - y * rot_sin) * scale,
            (x * rot_sin + y * rot_cos) * scale,
        ]
    };

    let mut prev = point(0);
    for k in 1..=drawn {
        let cur = point(k);
        out.push(SegmentInstance {
            a: prev,
            b: cur,
            color,
            width,
        });
        prev = cur;
    }
}

#[cfg(test)]
mod tests {
    // Test asserts use indexing on the produced Vec; allowed here over the
    // file's hot-path pragma since test code is not the render path.
    #![allow(clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn rose_is_deterministic_and_capped() {
        let mut a = Vec::with_capacity(400);
        let mut b = Vec::with_capacity(400);
        maurer_rose(6.0, 71.0, 360, 1.0, 0.0, 1.0, [1.0, 1.0, 1.0], 0.01, &mut a);
        maurer_rose(6.0, 71.0, 360, 1.0, 0.0, 1.0, [1.0, 1.0, 1.0], 0.01, &mut b);
        assert_eq!(a, b, "same parameters yield byte-identical geometry");
        // `samples` is the chord count when fully drawn.
        assert_eq!(a.len(), 360);
    }

    #[test]
    fn draw_progress_reveals_a_prefix() {
        let mut full = Vec::with_capacity(400);
        let mut half = Vec::with_capacity(400);
        maurer_rose(6.0, 71.0, 360, 1.0, 0.0, 1.0, [1.0; 3], 0.01, &mut full);
        maurer_rose(6.0, 71.0, 360, 1.0, 0.0, 0.5, [1.0; 3], 0.01, &mut half);
        assert!(half.len() < full.len(), "half progress draws fewer chords");
        // The drawn chords are a prefix of the full curve (same start).
        assert_eq!(half[0], full[0]);
    }

    #[test]
    fn sampling_into_a_preallocated_buffer_does_not_grow_it() {
        let mut out = Vec::with_capacity(512);
        let cap = out.capacity();
        for frame in 0..8 {
            let rotation = frame as f32 * 0.1;
            maurer_rose(
                5.0, 97.0, 361, 0.8, rotation, 1.0, [0.5; 3], 0.008, &mut out,
            );
        }
        assert_eq!(out.capacity(), cap, "resampling reused the buffer capacity");
    }
}
