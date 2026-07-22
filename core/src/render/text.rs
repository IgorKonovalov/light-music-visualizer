//! On-canvas text via glyphon (ADR-0009), behind the non-default `text` feature.
//!
//! A small, reusable seam: the frontend queues a list of positioned [`TextRun`]s
//! each frame; [`TextLayer`] shapes them and draws them in a second render pass
//! that loads (does not clear) the scene, so text composites over the visual in
//! the same frame. It lives in `core` — not the standalone — because that is
//! where the wgpu device/queue/surface live (ADR-0001: the frontend never sees a
//! backend); the `text` **feature**, not a crate boundary, keeps it out of the
//! plugin/default build. First consumer is Plan 0008's browse overlay; Plan
//! 0009's HUD reuses the same seam rather than a throwaway.

// Hot-path panic-denial pragma (Plan 0002 Phase 2; `render/` scan set). Runs
// every displayed frame while text is queued; a panic here is a visible crash.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

use glyphon::{
    Attrs, Buffer, Cache, Color, Family, FontSystem, Metrics, Resolution, Shaping, SwashCache,
    TextArea, TextAtlas, TextBounds, TextRenderer, Viewport,
};

/// A single positioned run of text the frontend queues for the current frame.
/// Coordinates are top-left device pixels (matching the diagnostics overlay);
/// `color` is linear RGBA in `0.0..=1.0`. The public seam the overlay and a
/// later HUD both fill.
pub struct TextRun<'a> {
    /// The text to draw (a single line; no wrapping is applied).
    pub text: &'a str,
    /// Left edge, device pixels from the surface's top-left.
    pub x: f32,
    /// Top edge, device pixels from the surface's top-left.
    pub y: f32,
    /// Font size in device pixels.
    pub size: f32,
    /// Linear RGBA in `0.0..=1.0`.
    pub color: [f32; 4],
}

/// An owned copy of a queued run, held from [`TextLayer::queue`] until the flush
/// in `render()` — the caller's borrowed `&str` need not outlive its own frame.
struct OwnedRun {
    text: String,
    x: f32,
    y: f32,
    size: f32,
    color: [f32; 4],
}

/// Line height as a multiple of the font size. Runs are single-line, so this
/// only sets vertical extent, never wrapping.
const LINE_HEIGHT_RATIO: f32 = 1.25;

/// Owns glyphon's font/atlas/renderer state plus a reusable buffer pool, and the
/// per-frame queue of runs. One instance per [`super::Renderer`].
pub struct TextLayer {
    font_system: FontSystem,
    swash_cache: SwashCache,
    viewport: Viewport,
    atlas: TextAtlas,
    renderer: TextRenderer,
    /// Runs queued for the current frame (cleared each frame at `end_frame`).
    runs: Vec<OwnedRun>,
    /// One reusable cosmic-text buffer per run, grown on demand and reshaped in
    /// place each frame — no per-frame `Buffer` allocation in steady state.
    buffers: Vec<Buffer>,
    /// Whether the last `prepare` produced drawable content for `render`.
    ready: bool,
}

impl TextLayer {
    /// Build the text layer on `device`, targeting `format` (the surface format).
    /// Loads the system font set once here, not per frame.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, format: wgpu::TextureFormat) -> Self {
        let font_system = FontSystem::new();
        let swash_cache = SwashCache::new();
        let cache = Cache::new(device);
        let viewport = Viewport::new(device, &cache);
        let mut atlas = TextAtlas::new(device, queue, &cache, format);
        let renderer =
            TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None);
        Self {
            font_system,
            swash_cache,
            viewport,
            atlas,
            renderer,
            runs: Vec::new(),
            buffers: Vec::new(),
            ready: false,
        }
    }

    /// Replace this frame's queued runs with owning copies of `runs`.
    pub fn queue(&mut self, runs: &[TextRun<'_>]) {
        self.runs.clear();
        self.runs.extend(runs.iter().map(|r| OwnedRun {
            text: r.text.to_owned(),
            x: r.x,
            y: r.y,
            size: r.size,
            color: r.color,
        }));
    }

    /// Shape the queued runs and upload their glyphs to the atlas. Returns
    /// whether there is anything to draw; an atlas-full or shaping failure
    /// degrades to "nothing drawn" rather than panicking on the render path.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        width: u32,
        height: u32,
    ) -> bool {
        self.ready = false;
        if self.runs.is_empty() {
            return false;
        }

        // Split-borrow the fields so the buffer pool and the font system can be
        // mutated disjointly (glyphon's shaping needs both).
        let Self {
            font_system,
            swash_cache,
            viewport,
            atlas,
            renderer,
            runs,
            buffers,
            ready,
        } = self;

        // Grow the reusable pool to cover this frame's run count.
        while buffers.len() < runs.len() {
            buffers.push(Buffer::new(
                font_system,
                Metrics::new(16.0, 16.0 * LINE_HEIGHT_RATIO),
            ));
        }

        // Reshape one buffer per run with its size and text (single line).
        for (buf, run) in buffers.iter_mut().zip(runs.iter()) {
            buf.set_metrics(Metrics::new(run.size, run.size * LINE_HEIGHT_RATIO));
            buf.set_size(None, None); // no wrap; TextArea bounds clip to screen
            buf.set_text(
                run.text.as_str(),
                &Attrs::new().family(Family::SansSerif),
                Shaping::Advanced,
                None,
            );
            buf.shape_until_scroll(font_system, false);
        }

        viewport.update(
            queue,
            Resolution {
                width: width.max(1),
                height: height.max(1),
            },
        );

        let clip_w = width.max(1) as i32;
        let clip_h = height.max(1) as i32;
        let areas = buffers.iter().zip(runs.iter()).map(|(buf, run)| TextArea {
            buffer: buf,
            left: run.x,
            top: run.y,
            scale: 1.0,
            bounds: TextBounds {
                left: 0,
                top: 0,
                right: clip_w,
                bottom: clip_h,
            },
            default_color: color_of(run.color),
            custom_glyphs: &[],
        });

        if renderer
            .prepare(
                device,
                queue,
                font_system,
                atlas,
                viewport,
                areas,
                swash_cache,
            )
            .is_err()
        {
            return false; // atlas full / shaping error — skip text this frame
        }
        *ready = true;
        true
    }

    /// Draw the prepared runs into `pass` (a load pass over the scene). No-op if
    /// `prepare` produced nothing drawable.
    pub fn render<'pass>(&'pass self, pass: &mut wgpu::RenderPass<'pass>) {
        if !self.ready {
            return;
        }
        // Best-effort: a render error can't recover mid-frame, so drop it rather
        // than panic on the hot path (the text simply won't appear).
        let _ = self.renderer.render(&self.atlas, &self.viewport, pass);
    }

    /// End-of-frame housekeeping: free atlas space unused this frame and clear
    /// the queue for the next one.
    pub fn end_frame(&mut self) {
        self.atlas.trim();
        self.runs.clear();
        self.ready = false;
    }
}

/// Map a linear `[r, g, b, a]` in `0.0..=1.0` to glyphon's 8-bit color.
fn color_of([r, g, b, a]: [f32; 4]) -> Color {
    let to_u8 = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    Color::rgba(to_u8(r), to_u8(g), to_u8(b), to_u8(a))
}
