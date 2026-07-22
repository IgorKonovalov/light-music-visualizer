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
pub mod parametric;
pub mod renderer;

pub use parametric::ParametricCurveScene;
pub use renderer::{LineRenderer, SegmentInstance};

/// Fixed segment-buffer capacity for every line scene, tuned to the iGPU floor
/// (ADR-0007 Risks: ~20k). A curve's `samples` and a generator's structure are
/// both clamped to this, and any drop is surfaced at load — never a silent cut.
pub const MAX_SEGMENTS: usize = 20_000;

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
