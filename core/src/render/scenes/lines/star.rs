//! Star-pattern scene: a Hankin star rosette built and cached at preset load
//! (ADR-0007 generator build model), cheap to animate. `configure` precomputes
//! a small set of **contact-angle variants** off the hot path; per frame the
//! scene picks the active variant and applies a rotate/scale/colour/draw-on
//! transform (allocation-free). A beat can swap the active variant for a
//! structural accent; continuous params drive rotation, hue, and draw-on.

// Hot-path panic-denial pragma: `update`/`render` run every displayed frame.
// `configure` (the Hankin construction) is build-time but colocated, so it
// obeys the same panic-free bar.
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
    CapOverflow, GeneratorConfig, MAX_SEGMENTS, ViewTransform, hankin, palette, transform_cached,
    turtle,
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

/// Contact-angle offsets (degrees) for the precomputed variants a beat swaps
/// between — a pointier and a blunter star around the preset's base angle.
const VARIANT_OFFSETS_DEG: [f32; 3] = [-24.0, 0.0, 24.0];
/// Contact angle is clamped to this range for a sensible star.
const CONTACT_MIN_DEG: f32 = 8.0;
const CONTACT_MAX_DEG: f32 = 80.0;

const DEFAULT_VARIANT: f32 = 1.0;
const DEFAULT_ROTATION: f32 = 0.0;
const DEFAULT_HUE: f32 = 0.5;
const DEFAULT_DRAW_PROGRESS: f32 = 1.0;
const DEFAULT_THICKNESS: f32 = 2.0;
const DEFAULT_SCALE: f32 = 1.0;
const DEFAULT_BRIGHTNESS: f32 = 1.0;
// Shared view transform (ADR-0018): identity by default.
const DEFAULT_ZOOM: f32 = 1.0;
const DEFAULT_PAN: f32 = 0.0;

/// A generator scene drawing a Hankin star pattern.
pub struct StarPatternScene {
    /// The single line renderer, shared with the other line scenes (ADR-0007).
    renderer: Rc<RefCell<LineRenderer>>,
    /// One cached rosette per contact-angle variant, built in `configure`.
    cached: Vec<Vec<SegmentInstance>>,
    /// Reused per-frame draw buffer — preallocated so the transform allocates
    /// nothing on the hot path.
    draw_buf: Vec<SegmentInstance>,
    /// Shared scene clock (seconds).
    time: f32,
    variant: f32,
    rotation: f32,
    hue: f32,
    draw_progress: f32,
    thickness: f32,
    scale: f32,
    brightness: f32,
    zoom: f32,
    pan_x: f32,
    pan_y: f32,
}

impl StarPatternScene {
    /// Build the scene over the shared line renderer, preallocating the draw
    /// buffer. No pattern is built until a preset configures one.
    pub fn new(renderer: Rc<RefCell<LineRenderer>>) -> Self {
        Self {
            renderer,
            cached: Vec::new(),
            draw_buf: Vec::with_capacity(MAX_SEGMENTS),
            time: 0.0,
            variant: DEFAULT_VARIANT,
            rotation: DEFAULT_ROTATION,
            hue: DEFAULT_HUE,
            draw_progress: DEFAULT_DRAW_PROGRESS,
            thickness: DEFAULT_THICKNESS,
            scale: DEFAULT_SCALE,
            brightness: DEFAULT_BRIGHTNESS,
            zoom: DEFAULT_ZOOM,
            pan_x: DEFAULT_PAN,
            pan_y: DEFAULT_PAN,
        }
    }

    /// Build + cache one rosette per contact-angle variant. Off the hot path.
    fn build(&mut self, n: u32, contact_angle_deg: f32) {
        self.cached.clear();
        for offset in VARIANT_OFFSETS_DEG {
            let ang = (contact_angle_deg + offset)
                .clamp(CONTACT_MIN_DEG, CONTACT_MAX_DEG)
                .to_radians();
            let mut segs = Vec::new();
            hankin::star_rosette(n, ang, &mut segs);
            turtle::normalize_fit(&mut segs, 0.9);
            self.cached.push(segs);
        }
    }
}

impl Scene for StarPatternScene {
    fn name(&self) -> &'static str {
        "star pattern"
    }

    fn set_time(&mut self, time: f32) {
        self.time = time;
    }

    fn reset_params(&mut self) {
        self.variant = DEFAULT_VARIANT;
        self.rotation = DEFAULT_ROTATION;
        self.hue = DEFAULT_HUE;
        self.draw_progress = DEFAULT_DRAW_PROGRESS;
        self.thickness = DEFAULT_THICKNESS;
        self.scale = DEFAULT_SCALE;
        self.brightness = DEFAULT_BRIGHTNESS;
        self.zoom = DEFAULT_ZOOM;
        self.pan_x = DEFAULT_PAN;
        self.pan_y = DEFAULT_PAN;
    }

    fn set_param(&mut self, name: &str, value: f32) {
        match name {
            "variant" => self.variant = value,
            "rotation" => self.rotation = value,
            "hue" => self.hue = value,
            "draw_progress" => self.draw_progress = value,
            "thickness" => self.thickness = value,
            "scale" => self.scale = value,
            "brightness" => self.brightness = value,
            "zoom" => self.zoom = value,
            "pan_x" => self.pan_x = value,
            "pan_y" => self.pan_y = value,
            _ => {}
        }
    }

    fn configure(&mut self, cfg: &GeneratorConfig) -> Option<CapOverflow> {
        // Build + cache the star variants off the hot path. Other config
        // variants belong to sibling line scenes and are ignored.
        match cfg {
            GeneratorConfig::Star {
                order,
                contact_angle_deg,
            } => self.build(*order, *contact_angle_deg),
            GeneratorConfig::Curve { .. }
            | GeneratorConfig::LSystem { .. }
            | GeneratorConfig::Particles { .. } => {}
        }
        // A rosette is `2 * n` segments for the small regular tilings v1 allows
        // (n <= 12), far under the cap — no truncation to surface.
        None
    }

    fn update(&mut self, _frame: &AnalysisFrame) {
        let variants = self.cached.len();
        if variants == 0 {
            self.draw_buf.clear();
            return;
        }
        let idx = (self.variant.max(0.0) as usize).min(variants.saturating_sub(1));
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
        let xform = ViewTransform {
            zoom: self.zoom,
            pan: [self.pan_x, self.pan_y],
            _pad: 0.0,
        };
        self.renderer.borrow_mut().draw(
            queue,
            encoder,
            view,
            aspect,
            1.0,
            CLEAR,
            xform,
            &self.draw_buf,
        );
    }
}
