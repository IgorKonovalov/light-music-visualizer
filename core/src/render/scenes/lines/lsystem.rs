//! L-system scene: expensive to build, cheap to animate (ADR-0007 generator
//! build model). At preset load (`configure`, off the hot path) the grammar is
//! expanded and turtle-walked into one cached segment buffer *per depth*
//! `1..=max_depth`. Per frame the scene only picks the visible depth and applies
//! a rotation / scale / colour / draw-on transform into the draw buffer — no
//! expansion, no allocation.
//!
//! Beat accents advance `visible_depth` (grow one iteration); continuous motion
//! drives `rotation`, `hue`, `draw_progress`, etc.

// Hot-path panic-denial pragma: `update`/`render` run every displayed frame.
// `configure` (expansion + turtle) is build-time but colocated, so it obeys the
// same panic-free bar.
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
use super::{
    GeneratorConfig, MAX_LSYSTEM_DEPTH, MAX_SEGMENTS, grammar, palette, transform_cached, turtle,
};
use crate::dsp::AnalysisFrame;

/// Maps `thickness` to an NDC-y half-width (see the parametric scene).
const WIDTH_SCALE: f32 = 0.003;

/// Near-black background so the additive glow reads.
const CLEAR: wgpu::Color = wgpu::Color {
    r: 0.01,
    g: 0.008,
    b: 0.02,
    a: 1.0,
};

const DEFAULT_VISIBLE_DEPTH: f32 = 1.0;
const DEFAULT_ROTATION: f32 = 0.0;
const DEFAULT_HUE: f32 = 0.3;
const DEFAULT_DRAW_PROGRESS: f32 = 1.0;
const DEFAULT_THICKNESS: f32 = 1.8;
const DEFAULT_SCALE: f32 = 1.0;
const DEFAULT_BRIGHTNESS: f32 = 1.0;

/// A generator scene driven by an L-system grammar.
pub struct LSystemScene {
    /// The single line renderer, shared with the other line scenes (ADR-0007).
    renderer: Rc<RefCell<LineRenderer>>,
    /// Base geometry per depth (index `d - 1`), built once in `configure`.
    /// Positions only; colour/width are applied per frame.
    cached: Vec<Vec<SegmentInstance>>,
    /// Reused per-frame draw buffer — preallocated so the transform allocates
    /// nothing on the hot path.
    draw_buf: Vec<SegmentInstance>,
    /// If a depth overflowed the segment cap at load: `(depth, dropped)`. Kept
    /// queryable rather than silently discarded (ADR-0007 cap is never silent);
    /// curated presets stay under the cap so this is normally `None`.
    overflow: Option<(u32, usize)>,
    /// Shared scene clock (seconds).
    time: f32,
    visible_depth: f32,
    rotation: f32,
    hue: f32,
    draw_progress: f32,
    thickness: f32,
    scale: f32,
    brightness: f32,
}

impl LSystemScene {
    /// Build the scene over the shared line renderer, preallocating the draw
    /// buffer. No grammar is expanded until a preset configures one.
    pub fn new(renderer: Rc<RefCell<LineRenderer>>) -> Self {
        Self {
            renderer,
            cached: Vec::new(),
            draw_buf: Vec::with_capacity(MAX_SEGMENTS),
            overflow: None,
            time: 0.0,
            visible_depth: DEFAULT_VISIBLE_DEPTH,
            rotation: DEFAULT_ROTATION,
            hue: DEFAULT_HUE,
            draw_progress: DEFAULT_DRAW_PROGRESS,
            thickness: DEFAULT_THICKNESS,
            scale: DEFAULT_SCALE,
            brightness: DEFAULT_BRIGHTNESS,
        }
    }

    /// The load-time cap overflow, if any: `(depth, segments dropped)`. A
    /// frontend/diagnostic can surface it; it is never silently swallowed.
    pub fn overflow(&self) -> Option<(u32, usize)> {
        self.overflow
    }

    /// Expand + turtle-walk each depth `1..=max_depth` into a cached buffer.
    /// Off the hot path (called from `configure`).
    fn build(&mut self, axiom: &str, rules: &[(char, String)], angle_deg: f32, max_depth: u32) {
        self.cached.clear();
        self.overflow = None;
        let depth = max_depth.clamp(1, MAX_LSYSTEM_DEPTH);
        let angle = angle_deg.to_radians();

        for d in 1..=depth {
            let string = grammar::expand(axiom, rules, d);
            let mut segs = Vec::new();
            let dropped = turtle::walk(&string, angle, MAX_SEGMENTS, &mut segs);
            turtle::normalize_fit(&mut segs, 0.9);
            if dropped > 0 && self.overflow.is_none() {
                self.overflow = Some((d, dropped));
            }
            self.cached.push(segs);
        }
    }
}

impl Scene for LSystemScene {
    fn name(&self) -> &'static str {
        "l-system"
    }

    fn set_time(&mut self, time: f32) {
        self.time = time;
    }

    fn reset_params(&mut self) {
        self.visible_depth = DEFAULT_VISIBLE_DEPTH;
        self.rotation = DEFAULT_ROTATION;
        self.hue = DEFAULT_HUE;
        self.draw_progress = DEFAULT_DRAW_PROGRESS;
        self.thickness = DEFAULT_THICKNESS;
        self.scale = DEFAULT_SCALE;
        self.brightness = DEFAULT_BRIGHTNESS;
    }

    fn set_param(&mut self, name: &str, value: f32) {
        match name {
            "visible_depth" => self.visible_depth = value,
            "rotation" => self.rotation = value,
            "hue" => self.hue = value,
            "draw_progress" => self.draw_progress = value,
            "thickness" => self.thickness = value,
            "scale" => self.scale = value,
            "brightness" => self.brightness = value,
            _ => {}
        }
    }

    fn configure(&mut self, cfg: &GeneratorConfig) {
        // Build + cache the grammar's geometry off the hot path. Other config
        // variants belong to sibling line scenes and are ignored.
        match cfg {
            GeneratorConfig::LSystem {
                axiom,
                rules,
                angle_deg,
                max_depth,
                seed: _,
            } => self.build(axiom, rules, *angle_deg, *max_depth),
            GeneratorConfig::Curve { .. } | GeneratorConfig::Star { .. } => {}
        }
    }

    fn update(&mut self, _frame: &AnalysisFrame) {
        // Pick the visible depth (1-based) and its cached base geometry.
        let depths = self.cached.len();
        if depths == 0 {
            self.draw_buf.clear();
            return;
        }
        let want = self.visible_depth.max(1.0) as usize;
        let idx = want.min(depths).saturating_sub(1);
        let Some(base) = self.cached.get(idx) else {
            self.draw_buf.clear();
            return;
        };

        let pal = palette(self.hue);
        let color = [
            pal[0] * self.brightness,
            pal[1] * self.brightness,
            pal[2] * self.brightness,
        ];
        let width = (self.thickness * WIDTH_SCALE).max(0.0005);
        transform_cached(
            base,
            self.rotation,
            self.scale,
            color,
            width,
            self.draw_progress,
            &mut self.draw_buf,
        );
    }

    fn render(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        aspect: f32,
    ) {
        self.renderer
            .borrow_mut()
            .draw(queue, encoder, view, aspect, 1.0, CLEAR, &self.draw_buf);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::indexing_slicing)]

    use super::*;

    /// A fixed base + repeated per-frame transforms must not grow the draw
    /// buffer — the per-frame half is allocation-free (ADR-0007). This is the
    /// "inspection" proof; expansion/turtle-walking live only in `build`.
    #[test]
    fn per_frame_transform_does_not_allocate() {
        let mut base = Vec::with_capacity(64);
        turtle::walk("F+F+F+F+F[-F]F", 0.5, MAX_SEGMENTS, &mut base);
        turtle::normalize_fit(&mut base, 0.9);

        let mut out = Vec::with_capacity(MAX_SEGMENTS);
        let cap = out.capacity();
        for frame in 0..16 {
            let rotation = frame as f32 * 0.05;
            transform_cached(&base, rotation, 1.0, [0.5; 3], 0.01, 1.0, &mut out);
        }
        assert_eq!(out.capacity(), cap, "per-frame transform reused the buffer");
        assert_eq!(out.len(), base.len(), "full progress draws every segment");
    }

    #[test]
    fn draw_progress_reveals_a_prefix() {
        let mut base = Vec::with_capacity(64);
        turtle::walk("FFFFFFFF", 0.0, MAX_SEGMENTS, &mut base);
        let mut out = Vec::with_capacity(64);
        transform_cached(&base, 0.0, 1.0, [1.0; 3], 0.01, 0.5, &mut out);
        assert_eq!(out.len(), 4, "half of eight segments");
    }
}
