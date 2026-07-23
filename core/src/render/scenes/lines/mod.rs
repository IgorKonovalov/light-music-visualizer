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

/// Declarative structural config a line scene consumes once at preset load
/// (ADR-0007): **not** expressions — the family / grammar / tiling the sampler
/// or generator builds from. Delivered through the optional
/// [`Scene::configure`](super::Scene::configure) hook, off the hot path.
/// Extended by later phases with the L-system and star-pattern variants.
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
