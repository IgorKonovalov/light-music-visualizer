//! Built-in scenes and the thin trait the renderer cycles through.
//!
//! Per ADR-0002 this stays crate-internal and minimal: it is the vocabulary
//! the future preset engine will drive, not a public extension point — no
//! plugin registration, no dynamic dispatch beyond what cycling needs.

// Hot-path panic-denial pragma (Plan 0002 Phase 2, extended to scenes by Plan
// 0003 Phase 0). Scene update/render run every displayed frame; a panic here
// is a visible crash mid-show.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

pub mod fragment_field;
pub mod lines;
pub mod reaction_diffusion;
pub mod swarm;

use crate::dsp::AnalysisFrame;

/// The `dt` (seconds) the C ABI's legacy `lmv_render` and the headless capture
/// primitives inject when a caller has no real elapsed time to supply — the
/// former fixed scene step, now demoted to a fallback (Plan 0014 Phase 2, ADR-0012).
/// The live frontends measure and inject real `dt` instead, so animation is
/// frame-rate-independent; capture uses this fixed value so a render is a pure
/// function of its inputs.
pub(crate) const FALLBACK_DT: f32 = 1.0 / 60.0;

/// One visual. `update` advances state from the analysis frame; `render` draws
/// with the state it has.
///
/// Both built-in systems (fragment field, swarm) are preset-driven and
/// implement the named-parameter surface — `set_time`, `reset_params`,
/// `set_param` — that the preset layer evaluates into per frame (ADR-0002). The
/// trait carries no-op defaults so a future non-parametric scene need not.
pub(crate) trait Scene {
    fn name(&self) -> &'static str;
    fn update(&mut self, frame: &AnalysisFrame);
    fn render(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        aspect: f32,
    );

    /// Advance simulation state by `dt` real seconds (Plan 0014 Phase 2). The
    /// renderer injects the elapsed time each frame; a feedback scene steps its
    /// fixed-timestep accumulator here and a CPU-integrated scene (the swarm)
    /// scales its motion by `dt`, so both look identical over wall-clock time on
    /// any refresh rate. Stateless, purely `time`-driven scenes ignore it.
    fn advance(&mut self, _dt: f32) {}

    /// Set the shared scene clock (seconds). The renderer owns the single clock
    /// so an expression's `time` and the system's animation never diverge.
    fn set_time(&mut self, _time: f32) {}
    /// Reset every named parameter to its default (called each frame before the
    /// active preset's bindings are applied, so unbound params don't leak).
    fn reset_params(&mut self) {}
    /// Apply one named parameter; unknown names are ignored.
    fn set_param(&mut self, _name: &str, _value: f32) {}

    /// Consume a preset's declarative structural config (ADR-0007). Invoked
    /// **once at preset load, off the hot path** — a generator builds and caches
    /// its geometry here; a parametric scene records its family. Default no-op,
    /// so non-line scenes (fragment field, swarm) never implement it. The one
    /// optional widening of this trait ADR-0007 sanctions — keep it to this.
    ///
    /// Returns [`Some`](lines::CapOverflow) when building the geometry hit the
    /// segment cap and truncated, so the frontend can surface it — the cap is
    /// never a silent cut (ADR-0007 Risks). `None` means it fit (the norm).
    fn configure(&mut self, _cfg: &lines::GeneratorConfig) -> Option<lines::CapOverflow> {
        None
    }
}

/// The registry: every built-in scene, in cycling order. All scenes are
/// created up front so switching mid-show is an index bump, never a hitch.
pub(crate) fn create_all(
    device: &wgpu::Device,
    surface_format: wgpu::TextureFormat,
) -> Vec<Box<dyn Scene>> {
    // One shared line renderer for every line scene (ADR-0007: "one line
    // renderer"). A single instanced-quad pipeline + segment buffer, borrowed by
    // whichever line scene is active — only one draws per frame. (Two separate
    // line pipelines with byte-identical vertex layouts also mis-render on the
    // DX12 WARP software adapter the capture tests use; one renderer avoids it.)
    let line_renderer = std::rc::Rc::new(std::cell::RefCell::new(lines::LineRenderer::new(
        device,
        surface_format,
        lines::MAX_SEGMENTS,
        "lines",
    )));
    vec![
        Box::new(fragment_field::FragmentFieldScene::new(
            device,
            surface_format,
        )),
        Box::new(swarm::SwarmScene::new(device, surface_format)),
        Box::new(lines::ParametricCurveScene::new(line_renderer.clone())),
        Box::new(lines::LSystemScene::new(line_renderer.clone())),
        Box::new(lines::StarPatternScene::new(line_renderer.clone())),
        Box::new(reaction_diffusion::ReactionDiffusionScene::new(
            device,
            surface_format,
        )),
    ]
}

/// Tiny deterministic RNG (splitmix64) so visual randomness is explicitly
/// seeded (NFR 6) without pulling a rand crate.
pub(crate) struct SeededRng(u64);

impl SeededRng {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    pub(crate) fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// Uniform in [lo, hi).
    pub(crate) fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next_f32()
    }
}
