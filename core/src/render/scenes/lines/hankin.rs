//! Hankin star patterns: build an n-fold star rosette by the contact-angle
//! method. `n` contact points sit symmetrically on a circle; from each, a ray
//! leaves at the contact angle from the inward normal, and adjacent rays meet at
//! the petal tips. Connecting each contact point to its two neighbouring tips
//! traces the interlaced star. A build-time step (runs inside `Scene::configure`,
//! off the hot path).
//!
//! v1 scope (ADR-0007 / plan Risks): a small set of regular n-fold stars with a
//! contact angle — not arbitrary tessellations. The construction is a pure
//! deterministic function of `(n, contact_angle)` and, by building every petal
//! from the same rotation-equivariant rule, its segment set is invariant under a
//! `2*pi/n` rotation (directly unit-tested).

// Under render/, so it carries the panic pragma even though it runs only at
// preset load. Written panic-free.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

use std::f32::consts::TAU;

use super::renderer::SegmentInstance;

/// Map a `tiling` name to its star order `n`. Accepts a few named/numeric
/// regular tilings (the v1 set); returns `None` for anything else so the loader
/// can reject it.
pub fn tiling_order(tiling: &str) -> Option<u32> {
    Some(match tiling.trim() {
        "square" | "4" | "4.4.4.4" => 4,
        "hexagon" | "6" | "6.6.6" => 6,
        "octagon" | "8" | "4.8.8" => 8,
        "dodecagon" | "12" | "3.12.12" => 12,
        _ => return None,
    })
}

/// Intersect ray `p + t*d` with ray `q + s*e` (t, s unbounded — infinite lines).
/// `None` if near-parallel.
fn line_intersect(p: [f32; 2], d: [f32; 2], q: [f32; 2], e: [f32; 2]) -> Option<[f32; 2]> {
    let denom = d[0] * e[1] - d[1] * e[0];
    if denom.abs() < 1e-6 {
        return None;
    }
    let t = ((q[0] - p[0]) * e[1] - (q[1] - p[1]) * e[0]) / denom;
    Some([p[0] + t * d[0], p[1] + t * d[1]])
}

/// Build an `n`-fold star rosette with the given `contact_angle` (radians) into
/// `out` (cleared first). Produces `2 * n` segments when every petal tip
/// resolves. Positions are in roughly the unit disc; the scene fit-normalizes.
pub fn star_rosette(n: u32, contact_angle: f32, out: &mut Vec<SegmentInstance>) {
    out.clear();
    if n < 3 {
        return;
    }
    let nf = n as f32;

    // Contact point k, evenly spaced on the unit circle.
    let contact = |k: i32| -> [f32; 2] {
        let a = TAU * (k as f32) / nf;
        [a.cos(), a.sin()]
    };
    // Rotate a vector by `ang` radians.
    let rotate = |v: [f32; 2], ang: f32| -> [f32; 2] {
        let (s, c) = ang.sin_cos();
        [v[0] * c - v[1] * s, v[0] * s + v[1] * c]
    };

    for k in 0..n as i32 {
        let m0 = contact(k);
        let m1 = contact(k + 1);
        // Inward normals (toward the centre) — the contact points lie on the
        // unit circle, so the inward normal is just the negated position.
        let in0 = [-m0[0], -m0[1]];
        let in1 = [-m1[0], -m1[1]];
        // Adjacent rays lean toward each other at the contact angle and meet at
        // the petal tip between the two contact points. m0's ray tilts toward
        // m1 (clockwise off its inward normal); m1's tilts back toward m0.
        let d0 = rotate(in0, -contact_angle);
        let d1 = rotate(in1, contact_angle);
        if let Some(tip) = line_intersect(m0, d0, m1, d1) {
            out.push(seg(m0, tip));
            out.push(seg(m1, tip));
        }
    }
}

fn seg(a: [f32; 2], b: [f32; 2]) -> SegmentInstance {
    SegmentInstance {
        a,
        b,
        color: [1.0, 1.0, 1.0],
        width: 0.01,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn tiling_names_map_to_orders() {
        assert_eq!(tiling_order("hexagon"), Some(6));
        assert_eq!(tiling_order("6.6.6"), Some(6));
        assert_eq!(tiling_order("8"), Some(8));
        assert_eq!(tiling_order("nonsense"), None);
    }

    #[test]
    fn rosette_has_the_expected_segment_count() {
        let mut out = Vec::new();
        star_rosette(6, 30f32.to_radians(), &mut out);
        // Two segments per petal (contact -> tip -> next contact).
        assert_eq!(out.len(), 12);

        let mut oct = Vec::new();
        star_rosette(8, 30f32.to_radians(), &mut oct);
        assert_eq!(oct.len(), 16);
    }

    #[test]
    fn rosette_is_invariant_under_a_2pi_over_n_rotation() {
        let n = 6u32;
        let mut out = Vec::new();
        star_rosette(n, 32f32.to_radians(), &mut out);
        assert!(!out.is_empty());

        let ang = TAU / n as f32;
        let (s, c) = ang.sin_cos();
        let rot = |p: [f32; 2]| [p[0] * c - p[1] * s, p[0] * s + p[1] * c];

        // Every segment, rotated by 2*pi/n, must match some original segment
        // (as an unordered endpoint pair) — the pattern has n-fold symmetry.
        for seg in &out {
            let ra = rot(seg.a);
            let rb = rot(seg.b);
            let matched = out.iter().any(|other| {
                (close(other.a, ra) && close(other.b, rb))
                    || (close(other.a, rb) && close(other.b, ra))
            });
            assert!(matched, "rotated segment has no image in the pattern");
        }
    }

    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
    }
}
