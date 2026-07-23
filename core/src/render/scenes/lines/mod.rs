//! Line-geometry scenes (ADR-0007): a line-art category built on one shared
//! [`LineRenderer`] (segments -> thick glowing instanced quads) and two build
//! models over it — a cheap **parametric** system sampled every frame (the
//! Maurer rose) and, from Phase 3, an expensive **generator** system built and
//! cached at preset load. Ported in spirit from the user's Maurer rose,
//! L-system, and Islamic-star sketches; none of that JavaScript is reused, only
//! the math.
//!
//! The renderer and the per-frame scene halves are hot-path; the generators
//! (grammar/turtle/Hankin, from later phases) run only at load. All files here
//! live under `render/` and so carry the panic pragma the hygiene guard scans
//! for recursively — the build-time files are written panic-free too.

// Hot-path panic-denial pragma (Plan 0002 Phase 2, extended to scenes by Plan
// 0003 Phase 0). `palette` may be called per frame.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

pub mod curves;
pub mod grammar;
pub mod hankin;
pub mod lsystem;
pub mod parametric;
pub mod renderer;
pub mod star;
pub mod turtle;

pub use lsystem::LSystemScene;
pub use parametric::ParametricCurveScene;
pub use renderer::{LineRenderer, SegmentInstance};
pub use star::StarPatternScene;

/// Fixed segment-buffer capacity for every line scene, tuned to the iGPU floor
/// (ADR-0007 Risks: ~20k). A curve's `samples` and a generator's structure are
/// both clamped to this, and any drop is surfaced at load — never a silent cut.
pub const MAX_SEGMENTS: usize = 20_000;

/// Hard clamp on L-system iteration depth, enforced at preset load. A branching
/// rule expands exponentially, so an unbounded `max_depth` would stall a preset
/// switch and blow the segment cap (ADR-0007 Risks). Curated presets stay well
/// under this; the turtle's own segment cap is the second backstop.
pub const MAX_LSYSTEM_DEPTH: u32 = 7;

/// Hard clamp on the geometry-mirror rotational order (Plan 0018 Phase 4). Beyond
/// a couple dozen the fold is visually indistinguishable and only multiplies
/// segment count toward the cap; a sane ceiling keeps a runaway `mirror_order`
/// expression from doing useless work before the [`MAX_SEGMENTS`] cap bites.
pub const MAX_MIRROR_ORDER: u32 = 24;

/// The shared camera transform every scene family applies (ADR-0018): a uniform
/// **zoom** about the frame centre, then a **pan**, in world space before the
/// aspect divide. Identity (`zoom = 1`, `pan = 0`) leaves geometry exactly where
/// a scene placed it, so a preset that binds none of `zoom`/`pan_x`/`pan_y` is
/// unchanged. `#[repr(C)]` + `Pod` so it uploads straight into a line-renderer
/// uniform slot. Rotate is reserved for a follow-up (ADR-0018 reserves it).
///
/// Defined here for Phase 1 (the line scenes are the walking skeleton); Phase 2
/// threads the same transform through the fragment and swarm scenes.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ViewTransform {
    /// Uniform scale about the frame centre (`1.0` = no zoom).
    pub zoom: f32,
    /// Pan offset in world units `(x, y)`, applied after the zoom.
    pub pan: [f32; 2],
    /// Padding to fill a 16-byte uniform slot (unused).
    pub _pad: f32,
}

impl Default for ViewTransform {
    /// The identity view: no zoom, no pan.
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: [0.0, 0.0],
            _pad: 0.0,
        }
    }
}

/// Which parametric curve family a `[curve]` preset draws. Extend as Plan 0010's
/// follow-ups add curve families (epicycloids, Lissajous, ...); unknown names
/// are rejected at load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveFamily {
    /// The Maurer rose — `sin(n * theta)` walked at a fixed angular step.
    MaurerRose,
}

impl CurveFamily {
    /// Parse a `[curve] family` name, or `None` if unknown.
    pub fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "maurer_rose" => CurveFamily::MaurerRose,
            _ => return None,
        })
    }
}

/// Declarative structural config a scene consumes once at preset load
/// (ADR-0007): **not** expressions — the family / grammar / tiling the sampler
/// or generator builds from. Delivered through the optional
/// [`Scene::configure`](super::Scene::configure) hook, off the hot path. This is
/// the shared structural-config enum for every scene that has one: the line
/// scenes' curve/L-system/star variants, plus the compute-particle attractor
/// family (Plan 0016) — it is not line-specific despite living here.
#[derive(Debug, Clone)]
pub enum GeneratorConfig {
    /// A parametric curve: which family to sample.
    Curve {
        /// The curve family (Maurer rose, ...).
        family: CurveFamily,
    },
    /// An L-system: a grammar the generator expands and turtle-walks at load,
    /// caching one segment buffer per depth.
    LSystem {
        /// The starting string.
        axiom: String,
        /// Production rules `(predecessor, successor)`.
        rules: Vec<(char, String)>,
        /// Turn angle in degrees for `+`/`-`.
        angle_deg: f32,
        /// Iterations to precompute (`1..=max_depth`), clamped to
        /// [`MAX_LSYSTEM_DEPTH`] at load.
        max_depth: u32,
        /// Reserved seed for future stochastic rules; deterministic today.
        seed: u64,
    },
    /// A Hankin star pattern: an `n`-fold star rosette built at load, with a few
    /// contact-angle variants a beat can switch between.
    Star {
        /// Star order `n` (from the tiling), e.g. 6 or 12.
        order: u32,
        /// Contact angle in degrees; variants are precomputed around it.
        contact_angle_deg: f32,
    },
    /// A GPU compute-particle attractor (Plan 0016): which strange-attractor map
    /// the compute step iterates. Not a line scene — reuses this shared enum so
    /// the family rides the existing `configure` hook (no new trait method).
    Particles {
        /// The attractor family (De Jong, Clifford, Thomas, Lorenz).
        family: super::particles::AttractorFamily,
    },
}

/// Reported by [`Scene::configure`](super::super::Scene::configure) when
/// building a line scene's geometry hit the fixed [`MAX_SEGMENTS`] cap and
/// truncated. The cap must never be a silent cut (ADR-0007 Risks): `configure`
/// returns this so the frontend surfaces it at load. Produced off the hot path
/// (preset load only); `None` is the normal case where geometry fit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapOverflow {
    /// How many draw segments were dropped at the cap.
    pub dropped: usize,
    /// Where the drop happened, for the surfaced message (e.g. `"depth 6"`).
    pub context: String,
}

impl std::fmt::Display for CapOverflow {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "geometry exceeded the {}-segment cap at {} (dropped {} segment(s)); \
             reduce the structure or its depth",
            MAX_SEGMENTS, self.context, self.dropped
        )
    }
}

/// iq-style cosine palette (RGB phase-shifted), matching the swarm/fragment
/// scenes so line art shares the engine's colour language.
pub fn palette(t: f32) -> [f32; 3] {
    let tau = std::f32::consts::TAU;
    [
        0.5 + 0.5 * (tau * (t + 0.10)).cos(),
        0.5 + 0.5 * (tau * (t + 0.42)).cos(),
        0.5 + 0.5 * (tau * (t + 0.62)).cos(),
    ]
}

/// The per-frame half shared by every **generator** line scene (L-system,
/// star): transform cached base geometry into `out` — rotate by `rotation`
/// (radians), scale, colour, set `width`, and reveal a `progress` prefix
/// (line-draw-on). Allocation-free into a preallocated `out`; expansion /
/// construction lives at load, this is the only per-frame work.
pub(crate) fn transform_cached(
    base: &[SegmentInstance],
    rotation: f32,
    scale: f32,
    color: [f32; 3],
    width: f32,
    progress: f32,
    out: &mut Vec<SegmentInstance>,
) {
    out.clear();
    let (sin, cos) = rotation.sin_cos();
    let keep = ((base.len() as f32) * progress.clamp(0.0, 1.0)).round() as usize;
    let rot = |p: [f32; 2]| -> [f32; 2] {
        [
            (p[0] * cos - p[1] * sin) * scale,
            (p[0] * sin + p[1] * cos) * scale,
        ]
    };
    for seg in base.iter().take(keep) {
        out.push(SegmentInstance {
            a: rot(seg.a),
            b: rot(seg.b),
            color,
            width,
        });
    }
}

/// N-fold geometry-mirror spec (Plan 0018 Phase 4): replicate a line scene's
/// segment set under rotational (and optionally reflective) symmetry to build a
/// true geometric fractal. Driven by the `mirror_order` / `mirror_reflect` named
/// params. `order = 1, reflect = false` is the identity — the base drawn once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MirrorSpec {
    /// Rotational symmetry order (`>= 1`).
    pub order: u32,
    /// Also emit a reflected copy per sector (dihedral symmetry).
    pub reflect: bool,
}

impl MirrorSpec {
    /// Build a spec from the raw `mirror_order` / `mirror_reflect` param values —
    /// the shared conversion every line scene uses. The order rounds and clamps to
    /// `1..=MAX_MIRROR_ORDER` (a non-finite or `< 1` value is the identity);
    /// `reflect` is a `>= 0.5` threshold so a preset can drive it with a `beat`.
    pub fn from_params(order: f32, reflect: f32) -> Self {
        let order = if order.is_finite() {
            (order.round() as i64).clamp(1, MAX_MIRROR_ORDER as i64) as u32
        } else {
            1
        };
        Self {
            order,
            reflect: reflect >= 0.5,
        }
    }

    /// How many copies of the base a full replication emits.
    fn copies(self) -> usize {
        self.order.max(1) as usize * if self.reflect { 2 } else { 1 }
    }
}

/// Replicate `single` (already positioned/coloured segments) about the frame
/// centre under `mirror.order`-fold rotation, plus an optional reflected copy per
/// sector, into `out` (cleared first) — a geometric kaleidoscope whose segment
/// set is invariant under a `2*pi/order` rotation. Truncates at `cap` (the
/// renderer's [`MAX_SEGMENTS`]) and returns the number of segments dropped, so the
/// caller can surface it — the cap is never a silent cut (ADR-0007 Risks).
///
/// Allocation-free into a preallocated `out`; the per-frame half of every mirrored
/// line scene.
pub(crate) fn replicate_mirror(
    single: &[SegmentInstance],
    mirror: MirrorSpec,
    cap: usize,
    out: &mut Vec<SegmentInstance>,
) -> usize {
    out.clear();
    let n = mirror.order.max(1);
    let wanted = single.len() * mirror.copies();
    for k in 0..n {
        let sector = std::f32::consts::TAU * (k as f32) / (n as f32);
        let (sin, cos) = sector.sin_cos();
        for reflected in [false, true] {
            if reflected && !mirror.reflect {
                continue;
            }
            // Reflect across the x-axis (optional), then rotate into the sector.
            let map = |p: [f32; 2]| -> [f32; 2] {
                let y = if reflected { -p[1] } else { p[1] };
                [p[0] * cos - y * sin, p[0] * sin + y * cos]
            };
            for seg in single {
                if out.len() >= cap {
                    break;
                }
                out.push(SegmentInstance {
                    a: map(seg.a),
                    b: map(seg.b),
                    color: seg.color,
                    width: seg.width,
                });
            }
        }
    }
    wanted.saturating_sub(out.len())
}

#[cfg(test)]
mod tests {
    // Test asserts index the produced Vec; allowed here over the file's hot-path
    // pragma since test code is not the render path.
    #![allow(clippy::indexing_slicing)]

    use super::*;

    fn seg(a: [f32; 2], b: [f32; 2]) -> SegmentInstance {
        SegmentInstance {
            a,
            b,
            color: [0.4, 0.7, 1.0],
            width: 0.01,
        }
    }

    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
    }

    /// A 6-fold mirror of an asymmetric base must be invariant under a `2*pi/6`
    /// rotation (the Hankin-style symmetry proof) and emit exactly `order` copies.
    #[test]
    fn mirror_is_invariant_under_a_2pi_over_order_rotation() {
        // A deliberately asymmetric little scribble so symmetry is non-trivial.
        let single = vec![seg([0.1, 0.05], [0.4, 0.2]), seg([0.4, 0.2], [0.3, 0.5])];
        let order = 6u32;
        let mut out = Vec::new();
        let dropped = replicate_mirror(
            &single,
            MirrorSpec {
                order,
                reflect: false,
            },
            10_000,
            &mut out,
        );
        assert_eq!(dropped, 0, "well under the cap");
        assert_eq!(
            out.len(),
            single.len() * order as usize,
            "one rotated copy of the base per sector"
        );

        let ang = std::f32::consts::TAU / order as f32;
        let (s, c) = ang.sin_cos();
        let rot = |p: [f32; 2]| [p[0] * c - p[1] * s, p[0] * s + p[1] * c];
        for seg in &out {
            let ra = rot(seg.a);
            let rb = rot(seg.b);
            let matched = out.iter().any(|other| {
                (close(other.a, ra) && close(other.b, rb))
                    || (close(other.a, rb) && close(other.b, ra))
            });
            assert!(matched, "rotated segment has no image in the mirrored set");
        }
    }

    /// The identity spec (`from_params(1, 0)`) copies the base through unchanged.
    #[test]
    fn identity_spec_copies_the_base_unchanged() {
        let single = vec![seg([0.2, 0.3], [0.5, 0.1])];
        let mut out = Vec::new();
        let dropped =
            replicate_mirror(&single, MirrorSpec::from_params(1.0, 0.0), 10_000, &mut out);
        assert_eq!(dropped, 0);
        assert_eq!(out.len(), 1);
        assert!(close(out[0].a, single[0].a) && close(out[0].b, single[0].b));
    }

    /// Reflection doubles the copy count and stays rotationally symmetric.
    #[test]
    fn reflection_doubles_the_copies() {
        let single = vec![seg([0.1, 0.2], [0.4, 0.3])];
        let mut out = Vec::new();
        replicate_mirror(
            &single,
            MirrorSpec {
                order: 5,
                reflect: true,
            },
            10_000,
            &mut out,
        );
        assert_eq!(out.len(), single.len() * 5 * 2, "rotation x reflection");
    }

    /// Exceeding `cap` truncates the output and reports the exact drop — the cap
    /// is never a silent cut (ADR-0007).
    #[test]
    fn overflow_truncates_and_reports_the_drop() {
        // 100 base segments, 6-fold = 600 wanted, capped at 250 -> 350 dropped.
        let single: Vec<_> = (0..100)
            .map(|i| seg([i as f32 * 0.001, 0.1], [0.2, i as f32 * 0.001]))
            .collect();
        let mut out = Vec::new();
        let dropped = replicate_mirror(
            &single,
            MirrorSpec {
                order: 6,
                reflect: false,
            },
            250,
            &mut out,
        );
        assert_eq!(out.len(), 250, "output is truncated at the cap");
        assert_eq!(dropped, 600 - 250, "the exact drop is reported");
    }
}
