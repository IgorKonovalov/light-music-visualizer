//! The render seam: take an [`AnalysisFrame`], drive the active preset's system,
//! draw one frame.
//!
//! The render loop is driven by the frontend at display cadence and is fully
//! decoupled from audio delivery — the ring buffer is the seam (CLAUDE.md).
//! Cycling moves between loaded presets (ADR-0002); each preset names a built-in
//! system and binds its parameters to expressions the renderer evaluates from
//! the analysis frame plus the shared scene clock.

// Hot-path panic-denial pragma (Plan 0002 Phase 2). Runs every displayed
// frame; a panic here is a visible crash mid-show.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

pub mod capture;
pub mod context;
pub mod overlay;
mod overlay_font;
pub mod scenes;
#[cfg(feature = "text")]
pub mod text;

use crate::diag::{Diag, Metrics};
use crate::dsp::AnalysisFrame;
use crate::preset::{Preset, SystemKind, Variables};
pub use capture::CaptureImage;
pub use context::{RenderContext, RenderError};
use overlay::Overlay;
use scenes::Scene;
#[cfg(feature = "text")]
use text::TextLayer;
#[cfg(feature = "text")]
pub use text::TextRun;

/// Assumed bytes-per-pixel for the swapchain GPU-byte estimate (the common
/// 8-bit RGBA/BGRA surface formats). An approximation, per ADR-0008.
const SWAPCHAIN_BYTES_PER_PIXEL: u64 = 4;
/// Fixed 2-image approximation for the swapchain GPU-byte estimate. wgpu exposes
/// no real image count, so this stays a constant decoupled from the context's
/// `desired_maximum_frame_latency` (also 2); the figure is a trend indicator,
/// not an exact footprint (ADR-0008).
const SWAPCHAIN_IMAGE_COUNT: u64 = 2;

/// A preset's system to its slot in the roster built by [`scenes::create_all`].
/// The roster holds only preset-addressed systems, in this slot order.
fn system_slot(system: SystemKind) -> usize {
    match system {
        SystemKind::FragmentField => 0,
        SystemKind::Swarm => 1,
    }
}

/// The loaded presets plus the active index — the pure, GPU-free part of
/// selection. Split out of [`Renderer`] so the addressing contract (names in
/// roster order, in-range select, out-of-range no-op) is unit-testable without a
/// surface, mirroring how the diagnostics stats are a pure type behind the GPU
/// [`Renderer`]. [`Renderer`]'s preset methods delegate here 1:1.
struct Roster {
    presets: Vec<Preset>,
    active: usize,
}

impl Roster {
    fn new(presets: Vec<Preset>) -> Self {
        Self { presets, active: 0 }
    }

    /// Replace the roster; reset `active` to the start if it now points past the
    /// end. An empty set is ignored — a directory that briefly reads empty or
    /// all-malformed leaves the last good roster rendering (NFR 10).
    fn set_presets(&mut self, presets: Vec<Preset>) {
        if presets.is_empty() {
            return;
        }
        self.presets = presets;
        if self.active >= self.presets.len() {
            self.active = 0;
        }
    }

    /// Advance to the next preset (wrapping); a no-op on an empty roster.
    fn cycle(&mut self) {
        if !self.presets.is_empty() {
            self.active = (self.active + 1) % self.presets.len();
        }
    }

    /// Set the active preset **iff** `index` is in range; an out-of-range index
    /// is a no-op — never a panic, never a wrap.
    fn select(&mut self, index: usize) {
        if index < self.presets.len() {
            self.active = index;
        }
    }

    /// The active preset, or `None` on an empty roster.
    fn active_preset(&self) -> Option<&Preset> {
        self.presets.get(self.active)
    }

    /// The active preset's name, or a placeholder on an empty roster.
    fn name(&self) -> &str {
        self.active_preset()
            .map(|p| p.name.as_str())
            .unwrap_or("no presets")
    }

    /// The loaded preset names in roster order.
    fn names(&self) -> impl Iterator<Item = &str> {
        self.presets.iter().map(|p| p.name.as_str())
    }
}

/// How to build a headless [`Renderer`] for capture (Plan 0013).
#[derive(Debug, Clone, Copy)]
pub struct HeadlessOptions {
    /// Offscreen render width in pixels.
    pub width: u32,
    /// Offscreen render height in pixels.
    pub height: u32,
    /// Force a fallback (software) adapter — WARP on DX12 — so captures
    /// rasterize identically across machines. Tests want this on.
    pub prefer_software: bool,
}

/// Owns the GPU context, the built-in systems, and the loaded presets; renders
/// one frame per call by evaluating the active preset into the active system.
pub struct Renderer {
    ctx: RenderContext,
    scenes: Vec<Box<dyn Scene>>,
    /// Loaded presets + the active index (pure selection state — see [`Roster`]).
    roster: Roster,
    /// Shared scene clock (seconds), advanced one fixed step per rendered frame.
    /// The single source for both an expression's `time` and system animation.
    time: f32,
    /// Runtime diagnostics: rolling frame-time stats + overlay flags (Plan 0011).
    diag: Diag,
    /// The debug overlay pass, painted only while `diag.overlay_enabled()`.
    overlay: Overlay,
    /// On-canvas text seam (browse overlay / HUD), standalone-only via the
    /// `text` feature (ADR-0009); absent from the plugin/default build.
    #[cfg(feature = "text")]
    text_layer: TextLayer,
}

impl Renderer {
    /// Build a renderer drawing into `target` (a safe window handle — the
    /// standalone path). Starts with the embedded default presets.
    pub fn new(
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        width: u32,
        height: u32,
    ) -> Result<Self, RenderError> {
        let ctx = RenderContext::new(target, width, height)?;
        let scenes = crate::render::scenes::create_all(&ctx.device, ctx.surface_format());
        let overlay = Overlay::new(&ctx.device, ctx.surface_format());
        #[cfg(feature = "text")]
        let text_layer = TextLayer::new(&ctx.device, &ctx.queue, ctx.surface_format());
        Ok(Self {
            ctx,
            scenes,
            roster: Roster::new(crate::preset::default_presets()),
            time: 0.0,
            diag: Diag::new(),
            overlay,
            #[cfg(feature = "text")]
            text_layer,
        })
    }

    /// Build a **headless** renderer that draws into offscreen textures instead
    /// of a window (Plan 0013 capture tooling). Same scenes, presets, and
    /// per-frame evaluation as the on-surface path — only the target differs.
    /// Starts with the embedded default presets.
    pub fn new_headless(opts: HeadlessOptions) -> Result<Self, RenderError> {
        let ctx = RenderContext::new_headless(opts.width, opts.height, opts.prefer_software)?;
        let scenes = crate::render::scenes::create_all(&ctx.device, ctx.surface_format());
        let overlay = Overlay::new(&ctx.device, ctx.surface_format());
        #[cfg(feature = "text")]
        let text_layer = TextLayer::new(&ctx.device, &ctx.queue, ctx.surface_format());
        Ok(Self {
            ctx,
            scenes,
            roster: Roster::new(crate::preset::default_presets()),
            time: 0.0,
            diag: Diag::new(),
            overlay,
            #[cfg(feature = "text")]
            text_layer,
        })
    }

    /// Renderer targeting a native Win32 window the host owns — the C ABI
    /// path (foobar2000 shim). Starts with the embedded default presets (no
    /// ABI surface for preset selection yet).
    ///
    /// # Safety
    /// `hwnd` must be a valid window handle that outlives this renderer.
    #[cfg(windows)]
    pub unsafe fn new_from_win32_hwnd(
        hwnd: std::num::NonZeroIsize,
        width: u32,
        height: u32,
    ) -> Result<Self, RenderError> {
        let target = wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: Some(wgpu::rwh::RawDisplayHandle::Windows(
                wgpu::rwh::WindowsDisplayHandle::new(),
            )),
            raw_window_handle: wgpu::rwh::RawWindowHandle::Win32(
                wgpu::rwh::Win32WindowHandle::new(hwnd),
            ),
        };
        let ctx = unsafe { RenderContext::new_unsafe(target, width, height) }?;
        let scenes = crate::render::scenes::create_all(&ctx.device, ctx.surface_format());
        let overlay = Overlay::new(&ctx.device, ctx.surface_format());
        #[cfg(feature = "text")]
        let text_layer = TextLayer::new(&ctx.device, &ctx.queue, ctx.surface_format());
        Ok(Self {
            ctx,
            scenes,
            roster: Roster::new(crate::preset::default_presets()),
            time: 0.0,
            diag: Diag::new(),
            overlay,
            #[cfg(feature = "text")]
            text_layer,
        })
    }

    /// Reconfigure the surface for a new window size.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.ctx.resize(width, height);
    }

    /// Replace the preset roster (the standalone's hot-reload path). An empty
    /// set is ignored so a preset directory that briefly reads empty — or whose
    /// files are all malformed — leaves the last good roster rendering (NFR 10).
    pub fn set_presets(&mut self, presets: Vec<Preset>) {
        self.roster.set_presets(presets);
    }

    /// Switch to the next preset; returns its name. Instant — every system is
    /// built at startup, so cycling never hitches a live show.
    pub fn cycle_preset(&mut self) -> &str {
        self.roster.cycle();
        self.preset_name()
    }

    /// The loaded preset names in roster order — the browse overlay's list
    /// source (Plan 0008). Selection addresses these by absolute index.
    pub fn preset_names(&self) -> impl Iterator<Item = &str> {
        self.roster.names()
    }

    /// Jump to the preset at `index` (its absolute position in
    /// [`preset_names`](Self::preset_names)); returns the now-active name. An
    /// out-of-range `index` is a no-op (never a panic, never a wrap), so a stale
    /// index from a shrunk hot-reloaded roster is harmless.
    pub fn select_preset(&mut self, index: usize) -> &str {
        self.roster.select(index);
        self.preset_name()
    }

    /// Make the preset named `name` active, returning whether it was found. A
    /// by-name lookup layered over [`preset_names`](Self::preset_names) +
    /// index-based [`select_preset`](Self::select_preset) (Plan 0013 capture
    /// tooling — distinct from 0008's by-index selection). An unknown name
    /// leaves the active preset unchanged.
    pub fn select_preset_by_name(&mut self, name: &str) -> bool {
        let Some(index) = self.preset_names().position(|n| n == name) else {
            return false;
        };
        self.select_preset(index);
        true
    }

    /// Queue text runs to composite over the next rendered frame; the queue is
    /// cleared after each `render`. The standalone fills it each frame with the
    /// active preset name and, while the browse overlay is open, its rows. A
    /// `text`-feature (standalone) path — the plugin/default build has no text.
    #[cfg(feature = "text")]
    pub fn queue_text(&mut self, runs: &[TextRun<'_>]) {
        self.text_layer.queue(runs);
    }

    /// Enable or disable rolling frame-time collection — the gated diagnostics
    /// clock read (Plan 0011). The standalone leaves this on so the title always
    /// shows live fps/p99; turning it off keeps the core fully clock-free.
    pub fn enable_diagnostics(&mut self, on: bool) {
        self.diag.set_collecting(on);
    }

    /// Turn the on-screen debug overlay on or off (off by default). Independent
    /// of collection, so the plugin can log metrics without painting the overlay.
    pub fn set_overlay(&mut self, on: bool) {
        self.diag.set_overlay(on);
    }

    /// Whether the debug overlay is currently painted.
    pub fn overlay_enabled(&self) -> bool {
        self.diag.overlay_enabled()
    }

    /// The current diagnostics snapshot (fps, p99, GPU bytes, …).
    pub fn metrics(&self) -> Metrics {
        self.diag.metrics()
    }

    /// Name of the currently active preset.
    pub fn preset_name(&self) -> &str {
        self.roster.name()
    }

    /// Name of the built-in system the active preset drives (e.g. the frontend
    /// shows it next to the preset name).
    pub fn active_system_name(&self) -> &'static str {
        self.roster
            .active_preset()
            .and_then(|p| self.scenes.get(system_slot(p.system)))
            .map(|scene| scene.name())
            .unwrap_or("")
    }

    /// Draw the current preset for this analysis frame. Lost/outdated surfaces
    /// self-heal by reconfiguring; timeouts/occlusion skip the frame; only a
    /// validation failure (a bug) bubbles up.
    pub fn render(&mut self, frame: &AnalysisFrame) -> Result<(), RenderError> {
        self.time += scenes::SCENE_DT;

        // Core-tracked GPU footprint: the swapchain dominates what the core
        // allocates. An approximation (ADR-0008), refreshed each frame so it
        // tracks resizes and Phase 6's swapchain trim.
        self.diag.set_gpu_bytes(
            self.ctx.config.width as u64
                * self.ctx.config.height as u64
                * SWAPCHAIN_BYTES_PER_PIXEL
                * SWAPCHAIN_IMAGE_COUNT,
        );

        let Some(surface_tex) = Self::acquire(&self.ctx)? else {
            self.diag.record_dropped(); // transient (timeout/occluded) — skip
            return Ok(());
        };
        let view = surface_tex
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("lmv-frame"),
            });

        let (width, height) = (self.ctx.config.width, self.ctx.config.height);
        let draw_calls = self.draw_frame(frame, &mut encoder, &view, width, height);

        self.ctx.queue.submit(std::iter::once(encoder.finish()));
        self.ctx.queue.present(surface_tex);

        // Free atlas glyphs unused this frame and clear the queue for the next.
        #[cfg(feature = "text")]
        self.text_layer.end_frame();

        self.diag.set_draw_calls(draw_calls);
        self.diag.record_frame();
        Ok(())
    }

    /// Record this frame's scene pass — plus the optional text and overlay
    /// passes — into `encoder`, drawing into `view` at `width`×`height`. Shared
    /// by the on-surface present path and headless capture; the caller owns
    /// acquire/submit/present (or the offscreen copy-back). Evaluates the active
    /// preset into the active system using the current scene clock
    /// (`self.time`, advanced by the caller — this does not touch it). Returns
    /// the draw-call count.
    fn draw_frame(
        &mut self,
        frame: &AnalysisFrame,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) -> u32 {
        let Self {
            ctx,
            scenes,
            roster,
            time,
            diag,
            overlay,
            #[cfg(feature = "text")]
            text_layer,
        } = self;

        let Some(preset) = roster.active_preset() else {
            return 0; // no presets loaded — nothing to draw
        };
        let Some(scene) = scenes.get_mut(system_slot(preset.system)) else {
            return 0;
        };

        // Evaluate the preset's bindings into the system's named parameters.
        let vars = Variables::new(
            frame.bass,
            frame.mid,
            frame.treb,
            frame.onset,
            if frame.beat { 1.0 } else { 0.0 },
            frame.bar,
            *time,
        );
        scene.set_time(*time);
        scene.reset_params();
        for binding in &preset.params {
            scene.set_param(&binding.name, binding.expr.eval(&vars));
        }
        scene.update(frame);

        let aspect = width as f32 / height.max(1) as f32;
        scene.render(&ctx.queue, encoder, view, aspect);

        // One scene pass, plus the optional text and overlay passes below.
        let mut draw_calls = 1u32;

        // On-canvas text (browse overlay / HUD): a second pass that loads the
        // scene and composites the queued runs on top, in the same frame
        // (ADR-0009). Standalone-only via the `text` feature; when both this and
        // the diagnostics overlay are on, the overlay draws last so it sits on
        // top of the text.
        #[cfg(feature = "text")]
        {
            if text_layer.prepare(&ctx.device, &ctx.queue, width, height) {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("lmv-text-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            // Load: composite over the scene already in the view.
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                text_layer.render(&mut pass);
                draw_calls += 1;
            }
        }

        if diag.overlay_enabled() {
            let metrics = diag.metrics();
            overlay.render(
                &ctx.queue,
                encoder,
                view,
                (width, height),
                metrics,
                diag.stats().samples().map(|s| s * 1000.0),
            );
            draw_calls += 1;
        }

        draw_calls
    }

    /// Advance the scene clock one step and capture that single frame into an
    /// offscreen texture, returning tight RGBA (Plan 0013). Off the hot path —
    /// blocks on GPU readback; never call it from a live loop.
    pub fn capture_frame(&mut self, frame: &AnalysisFrame) -> Result<CaptureImage, RenderError> {
        self.time += scenes::SCENE_DT;
        self.capture_at_clock(frame)
    }

    /// Draw the active preset for `frame` at the **current** clock into a fresh
    /// offscreen texture and read it back. Does not advance the clock, so
    /// callers that already stepped it share this. The whole path (clear → draw
    /// → copy → map) is deterministic for a given `(preset, frame, clock)`.
    fn capture_at_clock(&mut self, frame: &AnalysisFrame) -> Result<CaptureImage, RenderError> {
        let (width, height) = (self.ctx.config.width, self.ctx.config.height);
        let format = self.ctx.surface_format();
        let (texture, view) = capture::create_target(&self.ctx.device, format, width, height);
        let (buffer, padded_bpr) = capture::create_readback(&self.ctx.device, width, height);
        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("lmv-capture-frame"),
            });
        capture::record_clear(&mut encoder, &view);
        let _ = self.draw_frame(frame, &mut encoder, &view, width, height);
        capture::record_copy(&mut encoder, &texture, &buffer, padded_bpr, width, height);
        self.ctx.queue.submit(std::iter::once(encoder.finish()));

        #[cfg(feature = "text")]
        self.text_layer.end_frame();

        capture::read_back(&self.ctx.device, &buffer, width, height, padded_bpr)
    }

    /// Capture preset `name` after advancing it `frames` steps from a fixed
    /// initial state, driven by a single constant `frame` (Plan 0013). A **pure
    /// function** of `(name, frame, frames)`: the scenes are rebuilt so any
    /// stateful system (e.g. the seeded swarm particles) starts from its
    /// deterministic seed, and the scene clock resets to `0.0`, so the result is
    /// independent of any earlier capture. Errors if `name` is not in the
    /// roster. `frames` is treated as at least 1.
    pub fn capture_preset(
        &mut self,
        name: &str,
        frame: &AnalysisFrame,
        frames: u32,
    ) -> Result<CaptureImage, RenderError> {
        if !self.select_preset_by_name(name) {
            return Err(RenderError::UnknownPreset(name.to_string()));
        }
        // Reset simulation state to the deterministic seed and the clock to 0,
        // so the same (name, frame, frames) always yields identical pixels and
        // differential probes (Phase 3) isolate the stimulus, not history.
        self.scenes = scenes::create_all(&self.ctx.device, self.ctx.surface_format());
        self.time = 0.0;

        let (width, height) = (self.ctx.config.width, self.ctx.config.height);
        let format = self.ctx.surface_format();
        let (texture, view) = capture::create_target(&self.ctx.device, format, width, height);

        // Warm the scene through the first frames-1 steps (state advances, pixels
        // discarded); then capture the final frame.
        let n = frames.max(1);
        for _ in 1..n {
            self.time += scenes::SCENE_DT;
            self.step_offscreen(frame, &view, width, height);
        }
        self.time += scenes::SCENE_DT;

        let (buffer, padded_bpr) = capture::create_readback(&self.ctx.device, width, height);
        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("lmv-capture-preset"),
            });
        capture::record_clear(&mut encoder, &view);
        let _ = self.draw_frame(frame, &mut encoder, &view, width, height);
        capture::record_copy(&mut encoder, &texture, &buffer, padded_bpr, width, height);
        self.ctx.queue.submit(std::iter::once(encoder.finish()));

        #[cfg(feature = "text")]
        self.text_layer.end_frame();

        capture::read_back(&self.ctx.device, &buffer, width, height, padded_bpr)
    }

    /// Draw one frame into `view` and submit it — advancing scene state without
    /// reading anything back. The warm-up step [`capture_preset`] uses to reach
    /// frame `N`.
    fn step_offscreen(
        &mut self,
        frame: &AnalysisFrame,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("lmv-capture-step"),
            });
        capture::record_clear(&mut encoder, view);
        let _ = self.draw_frame(frame, &mut encoder, view, width, height);
        self.ctx.queue.submit(std::iter::once(encoder.finish()));

        #[cfg(feature = "text")]
        self.text_layer.end_frame();
    }

    fn acquire(ctx: &RenderContext) -> Result<Option<wgpu::SurfaceTexture>, RenderError> {
        use wgpu::CurrentSurfaceTexture as C;
        let Some(surface) = ctx.surface.as_ref() else {
            return Ok(None); // headless context — no swapchain to present into
        };
        match surface.get_current_texture() {
            C::Success(t) | C::Suboptimal(t) => Ok(Some(t)),
            C::Timeout | C::Occluded => Ok(None),
            C::Outdated | C::Lost => {
                ctx.reconfigure();
                match surface.get_current_texture() {
                    C::Success(t) | C::Suboptimal(t) => Ok(Some(t)),
                    C::Validation => Err(RenderError::SurfaceValidation),
                    _ => Ok(None),
                }
            }
            C::Validation => Err(RenderError::SurfaceValidation),
        }
    }
}

#[cfg(test)]
mod tests {
    // The pure roster contract, tested without a GPU surface (a live `Renderer`
    // can't be built headlessly). The `Renderer::preset_names`/`select_preset`
    // wrappers delegate to `Roster` 1:1, so this covers the addressing contract
    // Plan 0008 Phase 2 names. Test asserts use `expect`, allowed here over the
    // file's hot-path panic-denial pragma — test code is not the render path.
    #![allow(clippy::expect_used, clippy::indexing_slicing)]

    use super::{HeadlessOptions, Renderer, Roster};
    use crate::dsp::AnalysisFrame;
    use crate::preset::Preset;

    /// A minimal valid preset: a known system + explicit name, no params.
    fn preset(name: &str) -> Preset {
        Preset::from_toml_str(&format!("system = \"swarm\"\nname = \"{name}\""))
            .expect("hand-written test preset is valid")
    }

    fn roster(names: &[&str]) -> Roster {
        Roster::new(names.iter().map(|n| preset(n)).collect())
    }

    #[test]
    fn names_are_yielded_in_roster_order() {
        let r = roster(&["alpha", "bravo", "charlie"]);
        let got: Vec<&str> = r.names().collect();
        assert_eq!(got, ["alpha", "bravo", "charlie"]);
    }

    #[test]
    fn select_addresses_by_absolute_index() {
        let mut r = roster(&["alpha", "bravo", "charlie"]);
        assert_eq!(r.name(), "alpha"); // a fresh roster starts at index 0
        r.select(2);
        assert_eq!(r.name(), "charlie"); // the third entry
    }

    #[test]
    fn out_of_range_select_is_a_no_op() {
        let mut r = roster(&["alpha", "bravo", "charlie"]);
        r.select(1);
        r.select(999); // past the end: unchanged — no panic, no wrap
        assert_eq!(r.name(), "bravo");
    }

    #[test]
    fn set_presets_clamps_active_when_the_roster_shrinks() {
        let mut r = roster(&["alpha", "bravo", "charlie"]);
        r.select(2);
        r.set_presets(vec![preset("solo")]); // index 2 now out of range
        assert_eq!(r.name(), "solo");
    }

    /// Phase 1 (Plan 0013): a surface-less renderer captures the active preset
    /// into an offscreen texture. `prefer_software` (WARP on DX12) keeps it
    /// reproducible on any adapter. Asserts a full tight RGBA buffer with at
    /// least one non-black pixel — the preset actually drew.
    #[test]
    fn headless_captures_a_non_black_frame() {
        let mut renderer = Renderer::new_headless(HeadlessOptions {
            width: 256,
            height: 256,
            prefer_software: true,
        })
        .expect("headless renderer builds on the software adapter");

        let img = renderer
            .capture_frame(&AnalysisFrame::default())
            .expect("capture succeeds");

        assert_eq!(img.width, 256);
        assert_eq!(img.height, 256);
        assert_eq!(img.rgba.len(), 256 * 256 * 4, "tight RGBA, no row padding");
        let non_black = img
            .rgba
            .chunks_exact(4)
            .any(|px| px[0] > 0 || px[1] > 0 || px[2] > 0);
        assert!(non_black, "the active preset drew at least one lit pixel");
    }

    /// Phase 2 (Plan 0013): `capture_preset` is a pure function of
    /// `(name, frame, frames)`. Uses the stateful swarm preset "Drift" — the
    /// case where a missing state reset would leak history — to prove two
    /// captures are byte-identical, that N=1 differs from N=120 (the scene
    /// animates), and that an unknown name is a clean error.
    #[test]
    fn capture_preset_is_deterministic_and_animates() {
        let mut renderer = Renderer::new_headless(HeadlessOptions {
            width: 128,
            height: 128,
            prefer_software: true,
        })
        .expect("headless renderer builds");
        let frame = AnalysisFrame::default();

        let a = renderer
            .capture_preset("Drift", &frame, 120)
            .expect("capture Drift @120");
        let b = renderer
            .capture_preset("Drift", &frame, 120)
            .expect("recapture Drift @120");
        assert_eq!(
            a.rgba, b.rgba,
            "same (preset, frame, N) is byte-identical across calls"
        );

        let one = renderer
            .capture_preset("Drift", &frame, 1)
            .expect("capture Drift @1");
        assert_ne!(
            one.rgba, a.rgba,
            "N=1 differs from N=120 — the scene advances over time"
        );

        assert!(
            renderer
                .capture_preset("no-such-preset", &frame, 1)
                .is_err(),
            "an unknown preset name is a clean error, not a panic"
        );
    }
}
