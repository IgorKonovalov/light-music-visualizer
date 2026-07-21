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
pub mod pulse;
pub mod spectrum;
pub mod starfield;
pub mod swarm;

use crate::dsp::AnalysisFrame;

/// Fixed per-frame timestep for scene animation. Frame-rate coupled by
/// design in the fixed-quality MVP; the quality-tier plan revisits this.
pub(crate) const SCENE_DT: f32 = 1.0 / 60.0;

/// One visual. `update` advances state from the analysis frame; `render` draws
/// with the state it has.
///
/// Preset-driven systems (fragment field, swarm) additionally implement the
/// named-parameter surface — `set_time`, `reset_params`, `set_param` — that the
/// preset layer evaluates into per frame (ADR-0002). Legacy scenes inherit the
/// no-op defaults and stay frame-driven.
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

    /// Set the shared scene clock (seconds). The renderer owns the single clock
    /// so an expression's `time` and the system's animation never diverge.
    fn set_time(&mut self, _time: f32) {}
    /// Reset every named parameter to its default (called each frame before the
    /// active preset's bindings are applied, so unbound params don't leak).
    fn reset_params(&mut self) {}
    /// Apply one named parameter; unknown names are ignored.
    fn set_param(&mut self, _name: &str, _value: f32) {}
}

/// The registry: every built-in scene, in cycling order. All scenes are
/// created up front so switching mid-show is an index bump, never a hitch.
pub(crate) fn create_all(
    device: &wgpu::Device,
    surface_format: wgpu::TextureFormat,
) -> Vec<Box<dyn Scene>> {
    vec![
        Box::new(fragment_field::FragmentFieldScene::new(
            device,
            surface_format,
        )),
        Box::new(swarm::SwarmScene::new(device, surface_format)),
        Box::new(spectrum::SpectrumScene::new(device, surface_format)),
        Box::new(pulse::PulseScene::new(device, surface_format)),
        Box::new(starfield::StarfieldScene::new(device, surface_format)),
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

/// Mean of the lowest bands — a serviceable bass proxy for scenes.
#[allow(
    clippy::indexing_slicing,
    reason = "n = 8 is a compile-time constant <= SPECTRUM_BINS (64), so the slice is always in-bounds"
)]
pub(crate) fn bass_level(frame: &AnalysisFrame) -> f32 {
    let n = 8;
    frame.spectrum[..n].iter().sum::<f32>() / n as f32
}

/// Mean over the full spectrum — overall energy.
pub(crate) fn energy_level(frame: &AnalysisFrame) -> f32 {
    frame.spectrum.iter().sum::<f32>() / frame.spectrum.len() as f32
}
