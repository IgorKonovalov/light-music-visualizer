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

use std::cell::RefCell;
use std::rc::Rc;

use super::super::Scene;
use super::renderer::{LineRenderer, SegmentInstance};
use super::{CapOverflow, CurveFamily, GeneratorConfig, MAX_SEGMENTS, curves, palette};
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

/// A parametric line curve (the Maurer rose), sampled per frame and driven by
/// named preset parameters over the audio analysis.
pub struct ParametricCurveScene {
    /// The single line renderer, shared with the other line scenes (ADR-0007:
    /// "one line renderer"). Only the active scene draws in a frame, so the
    /// shared pipeline + buffer are never contended.
    renderer: Rc<RefCell<LineRenderer>>,
    /// Reused segment buffer — preallocated to the cap so resampling never
    /// allocates on the hot path.
    segments: Vec<SegmentInstance>,
    /// Which curve family to sample, chosen at preset load via `configure`.
    family: CurveFamily,
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
    /// Build the scene over the shared line renderer, preallocating its segment
    /// buffer to the cap.
    pub fn new(renderer: Rc<RefCell<LineRenderer>>) -> Self {
        Self {
            renderer,
            segments: Vec::with_capacity(MAX_SEGMENTS),
            family: CurveFamily::MaurerRose,
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

    fn reset_params(&mut self) {
        self.n = DEFAULT_N;
        self.d = DEFAULT_D;
        self.samples = DEFAULT_SAMPLES;
        self.thickness = DEFAULT_THICKNESS;
        self.hue = DEFAULT_HUE;
        self.spin = DEFAULT_SPIN;
        self.scale = DEFAULT_SCALE;
        self.brightness = DEFAULT_BRIGHTNESS;
        self.draw_progress = DEFAULT_DRAW_PROGRESS;
    }

    fn set_param(&mut self, name: &str, value: f32) {
        match name {
            "n" => self.n = value,
            "d" => self.d = value,
            "samples" => self.samples = value,
            "thickness" => self.thickness = value,
            "hue" => self.hue = value,
            "spin" => self.spin = value,
            "scale" => self.scale = value,
            "brightness" => self.brightness = value,
            "draw_progress" => self.draw_progress = value,
            _ => {}
        }
    }

    fn configure(&mut self, cfg: &GeneratorConfig) -> Option<CapOverflow> {
        // A curve preset records its family here (off the hot path). Later
        // phases' generator config variants are for the generator scenes; this
        // match gains ignore-arms for them when they land.
        match cfg {
            GeneratorConfig::Curve { family } => self.family = *family,
            // Generator configs (L-system, star) belong to their own scenes.
            GeneratorConfig::LSystem { .. } | GeneratorConfig::Star { .. } => {}
        }
        // No load-time truncation: the parametric sampler builds nothing here.
        // Its only cap is a per-frame `samples` clamp in `update` (see there).
        None
    }

    fn update(&mut self, _frame: &AnalysisFrame) {
        // Per-frame defensive clamp: a huge `samples` can never overrun the
        // preallocated buffer (ADR-0007 cap is explicit). Unlike the generator
        // scenes' load-time build, `samples` is an expression evaluated every
        // frame, so there is no "load" moment to surface a truncation at, and a
        // sane curve preset (samples in the hundreds) never approaches the cap —
        // the clamp is a safety backstop, not a structural cut worth reporting.
        let samples = (self.samples.max(0.0) as usize).min(MAX_SEGMENTS);
        let rotation = self.spin * self.time;
        let base = palette(self.hue);
        let color = [
            base[0] * self.brightness,
            base[1] * self.brightness,
            base[2] * self.brightness,
        ];
        let width = (self.thickness * WIDTH_SCALE).max(0.0005);

        match self.family {
            CurveFamily::MaurerRose => curves::maurer_rose(
                self.n,
                self.d,
                samples,
                self.scale,
                rotation,
                self.draw_progress,
                color,
                width,
                &mut self.segments,
            ),
        }
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
            .borrow_mut()
            .draw(queue, encoder, view, aspect, 1.0, CLEAR, &self.segments);
    }
}
