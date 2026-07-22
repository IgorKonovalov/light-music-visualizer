//! Parametric-curve scene: a pure `t -> (x, y)` curve resampled every frame
//! into the shared [`LineRenderer`] (ADR-0007 parametric build model). Phase 1
//! is hardcoded to one Maurer rose that gently rotates on the deterministic
//! scene clock; Phase 2 makes the curve family and every named parameter
//! preset-driven so audio can sweep it live.

// Hot-path panic-denial pragma (Plan 0002 Phase 2, extended to scenes by Plan
// 0003 Phase 0). `update`/`render` run every displayed frame.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

use super::super::Scene;
use super::renderer::{LineRenderer, SegmentInstance};
use super::{MAX_SEGMENTS, curves, palette};
use crate::dsp::AnalysisFrame;

/// Maps the `thickness` parameter (a small integer-ish stroke weight) to an
/// NDC-y half-width; `thickness = 2` gives a comfortably thick projector line.
const WIDTH_SCALE: f32 = 0.003;

/// Background the curve is stroked over — a near-black so the additive glow
/// reads.
const CLEAR: wgpu::Color = wgpu::Color {
    r: 0.01,
    g: 0.008,
    b: 0.02,
    a: 1.0,
};

// Parameter defaults — a calm, whole, slowly turning rose when nothing is bound.
const DEFAULT_N: f32 = 6.0;
const DEFAULT_D: f32 = 71.0;
const DEFAULT_SAMPLES: f32 = 361.0;
const DEFAULT_THICKNESS: f32 = 2.0;
const DEFAULT_HUE: f32 = 0.6;
const DEFAULT_SPIN: f32 = 0.1;
const DEFAULT_SCALE: f32 = 0.9;
const DEFAULT_BRIGHTNESS: f32 = 1.0;
const DEFAULT_DRAW_PROGRESS: f32 = 1.0;

/// A parametric line curve (Phase 1: the Maurer rose), sampled per frame.
pub struct ParametricCurveScene {
    renderer: LineRenderer,
    /// Reused segment buffer — preallocated to the cap so resampling never
    /// allocates on the hot path.
    segments: Vec<SegmentInstance>,
    /// Shared scene clock (seconds), set by the renderer each frame.
    time: f32,
    n: f32,
    d: f32,
    samples: f32,
    thickness: f32,
    hue: f32,
    spin: f32,
    scale: f32,
    brightness: f32,
    draw_progress: f32,
}

impl ParametricCurveScene {
    /// Build the line pipeline and preallocate the segment buffer on `device`.
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        Self {
            renderer: LineRenderer::new(device, surface_format, MAX_SEGMENTS),
            segments: Vec::with_capacity(MAX_SEGMENTS),
            time: 0.0,
            n: DEFAULT_N,
            d: DEFAULT_D,
            samples: DEFAULT_SAMPLES,
            thickness: DEFAULT_THICKNESS,
            hue: DEFAULT_HUE,
            spin: DEFAULT_SPIN,
            scale: DEFAULT_SCALE,
            brightness: DEFAULT_BRIGHTNESS,
            draw_progress: DEFAULT_DRAW_PROGRESS,
        }
    }
}

impl Scene for ParametricCurveScene {
    fn name(&self) -> &'static str {
        "parametric curve"
    }

    fn set_time(&mut self, time: f32) {
        self.time = time;
    }

    fn update(&mut self, _frame: &AnalysisFrame) {
        // Sample count clamped to the buffer cap (minus one chord's headroom).
        let samples = (self.samples.max(0.0) as usize).min(MAX_SEGMENTS);
        let rotation = self.spin * self.time;
        let base = palette(self.hue);
        let color = [
            base[0] * self.brightness,
            base[1] * self.brightness,
            base[2] * self.brightness,
        ];
        let width = (self.thickness * WIDTH_SCALE).max(0.0005);

        curves::maurer_rose(
            self.n,
            self.d,
            samples,
            self.scale,
            rotation,
            self.draw_progress,
            color,
            width,
            &mut self.segments,
        );
    }

    fn render(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        aspect: f32,
    ) {
        // Segments carry brightness in their colour; glow multiplier stays 1.0.
        self.renderer
            .draw(queue, encoder, view, aspect, 1.0, CLEAR, &self.segments);
    }
}
