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

mod background;
pub mod capture;
pub mod context;
pub mod feedback;
pub mod metrics;
pub mod overlay;
mod overlay_font;
pub mod scenes;
#[cfg(feature = "text")]
pub mod text;

use crate::audio::AudioFormat;
use crate::diag::{Diag, Metrics};
use crate::dsp::AnalysisFrame;
use crate::preset::{Preset, SystemKind, Variables};
use background::Background;
pub use capture::CaptureImage;
pub use context::{RenderContext, RenderError};
use overlay::Overlay;
use scenes::Scene;
pub use scenes::lines::CapOverflow;
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
        SystemKind::ParametricCurve => 2,
        SystemKind::LSystem => 3,
        SystemKind::StarPattern => 4,
        SystemKind::ReactionDiffusion => 5,
        SystemKind::Attractor => 6,
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
    /// The background pre-pass (ADR-0018): fills the frame with a gradient/vignette
    /// backdrop before the scene draws. Driven by `bg_*` named params the renderer
    /// routes to it; owns the frame clear now that scenes `Load` instead of `Clear`.
    background: Background,
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
    /// Segment-cap truncation from the active preset's last `configure`, if any
    /// (ADR-0007: the cap is never a silent cut). Refreshed whenever the active
    /// preset changes; the frontend surfaces it. `None` when geometry fit.
    cap_overflow: Option<CapOverflow>,
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
        let background = Background::new(&ctx.device, ctx.surface_format());
        let overlay = Overlay::new(&ctx.device, ctx.surface_format());
        #[cfg(feature = "text")]
        let text_layer = TextLayer::new(&ctx.device, &ctx.queue, ctx.surface_format());
        let mut renderer = Self {
            ctx,
            scenes,
            background,
            roster: Roster::new(crate::preset::default_presets()),
            time: 0.0,
            diag: Diag::new(),
            overlay,
            #[cfg(feature = "text")]
            text_layer,
            cap_overflow: None,
        };
        // Apply the initial preset's structural config (ADR-0007) so a line
        // scene at roster index 0 renders with its geometry built.
        renderer.configure_active_scene();
        Ok(renderer)
    }

    /// Build a **headless** renderer that draws into offscreen textures instead
    /// of a window (Plan 0013 capture tooling). Same scenes, presets, and
    /// per-frame evaluation as the on-surface path — only the target differs.
    /// Starts with the embedded default presets.
    pub fn new_headless(opts: HeadlessOptions) -> Result<Self, RenderError> {
        let ctx = RenderContext::new_headless(opts.width, opts.height, opts.prefer_software)?;
        let scenes = crate::render::scenes::create_all(&ctx.device, ctx.surface_format());
        let background = Background::new(&ctx.device, ctx.surface_format());
        let overlay = Overlay::new(&ctx.device, ctx.surface_format());
        #[cfg(feature = "text")]
        let text_layer = TextLayer::new(&ctx.device, &ctx.queue, ctx.surface_format());
        let mut renderer = Self {
            ctx,
            scenes,
            background,
            roster: Roster::new(crate::preset::default_presets()),
            time: 0.0,
            diag: Diag::new(),
            overlay,
            #[cfg(feature = "text")]
            text_layer,
            cap_overflow: None,
        };
        // Apply the initial preset's structural config (ADR-0007) so a line
        // scene at roster index 0 renders with its geometry built.
        renderer.configure_active_scene();
        Ok(renderer)
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
        let background = Background::new(&ctx.device, ctx.surface_format());
        let overlay = Overlay::new(&ctx.device, ctx.surface_format());
        #[cfg(feature = "text")]
        let text_layer = TextLayer::new(&ctx.device, &ctx.queue, ctx.surface_format());
        let mut renderer = Self {
            ctx,
            scenes,
            background,
            roster: Roster::new(crate::preset::default_presets()),
            time: 0.0,
            diag: Diag::new(),
            overlay,
            #[cfg(feature = "text")]
            text_layer,
            cap_overflow: None,
        };
        // Apply the initial preset's structural config (ADR-0007) so a line
        // scene at roster index 0 renders with its geometry built.
        renderer.configure_active_scene();
        Ok(renderer)
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
        self.configure_active_scene();
    }

    /// Switch to the next preset; returns its name. Instant — every system is
    /// built at startup, so cycling never hitches a live show.
    pub fn cycle_preset(&mut self) -> &str {
        self.roster.cycle();
        self.configure_active_scene();
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
        self.configure_active_scene();
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

    /// Apply the active preset's declarative structural config to its scene, if
    /// it has one (ADR-0007). Called once whenever the active preset changes —
    /// on select/cycle/hot-reload and after a capture rebuilds the scenes — so a
    /// generator builds and caches its geometry exactly once, off the hot path.
    /// A `None` config (fragment/swarm, or a curve on the family default) is a
    /// no-op via the trait's default `configure`.
    fn configure_active_scene(&mut self) {
        let Self {
            scenes,
            roster,
            cap_overflow,
            ..
        } = self;
        *cap_overflow = None;
        let Some(preset) = roster.active_preset() else {
            return;
        };
        let Some(cfg) = preset.config.as_ref() else {
            return;
        };
        if let Some(scene) = scenes.get_mut(system_slot(preset.system)) {
            // Capture any segment-cap truncation so the frontend can surface it
            // (ADR-0007: never a silent cut). `None` for the fit/no-config case.
            *cap_overflow = scene.configure(cfg);
        }
    }

    /// The segment-cap truncation from the active preset's last `configure`, if
    /// its geometry hit the fixed cap (ADR-0007: the cap is never a silent cut).
    /// Refreshed on every active-preset change (select / cycle / hot-reload); the
    /// standalone surfaces it at load. `None` in the normal case where geometry
    /// fit — which is every shipped preset.
    pub fn cap_overflow(&self) -> Option<&CapOverflow> {
        // The configure-time overflow (an oversized L-system depth) takes
        // precedence; otherwise the active scene's per-frame geometry-mirror
        // overflow (Plan 0018 Phase 4), set once a frame has replicated. Both
        // reuse the same `CapOverflow` type so the frontend surfaces either.
        if let Some(overflow) = self.cap_overflow.as_ref() {
            return Some(overflow);
        }
        self.roster
            .active_preset()
            .and_then(|preset| self.scenes.get(system_slot(preset.system)))
            .and_then(|scene| scene.mirror_overflow())
    }

    /// Draw the current preset for this analysis frame, advancing all animation
    /// by `dt` real seconds (Plan 0014 Phase 2). The frontend measures and
    /// injects elapsed wall-clock time so the visuals run at the same speed on
    /// any refresh rate; `core` never reads a clock. Lost/outdated surfaces
    /// self-heal by reconfiguring; timeouts/occlusion skip the frame; only a
    /// validation failure (a bug) bubbles up.
    pub fn render(&mut self, frame: &AnalysisFrame, dt: f32) -> Result<(), RenderError> {
        self.time += dt;

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
        let draw_calls = self.draw_frame(frame, &mut encoder, &view, width, height, dt);

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
    /// (`self.time`, advanced by the caller — this does not touch it) and
    /// injects `dt` real seconds into the scene's [`advance`](scenes::Scene::advance)
    /// so its simulation steps at the same wall-clock rate on any refresh.
    /// Returns the draw-call count.
    fn draw_frame(
        &mut self,
        frame: &AnalysisFrame,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
        dt: f32,
    ) -> u32 {
        let Self {
            ctx,
            scenes,
            background,
            roster,
            time,
            diag,
            overlay,
            #[cfg(feature = "text")]
            text_layer,
            // Set at preset load, surfaced by the frontend — not a per-frame concern.
            cap_overflow: _,
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
        scene.advance(dt);
        background.reset_params();
        scene.reset_params();
        for binding in &preset.params {
            let value = binding.expr.eval(&vars);
            // Route `bg_*` params to the background pass; everything else to the
            // scene. The namespaces are disjoint, so no param reaches both.
            if !background.set_param(&binding.name, value) {
                scene.set_param(&binding.name, value);
            }
        }
        scene.update(frame);

        let aspect = width as f32 / height.max(1) as f32;
        // Fixed-order composite (ADR-0018): the backdrop first (it owns the clear),
        // then the active scene loads over it.
        background.render(&ctx.queue, encoder, view);
        scene.render(&ctx.queue, encoder, view, aspect);

        // Background + scene, plus the optional text and overlay passes below.
        let mut draw_calls = 2u32;

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
        self.time += scenes::FALLBACK_DT;
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
        let _ = self.draw_frame(
            frame,
            &mut encoder,
            &view,
            width,
            height,
            scenes::FALLBACK_DT,
        );
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
        self.background.reset_resources();
        self.time = 0.0;
        // The rebuilt scenes are fresh — re-apply the active preset's structural
        // config (ADR-0007) so a line scene captures with its geometry built.
        self.configure_active_scene();

        let (width, height) = (self.ctx.config.width, self.ctx.config.height);
        let format = self.ctx.surface_format();
        let (texture, view) = capture::create_target(&self.ctx.device, format, width, height);

        // Warm the scene through the first frames-1 steps (state advances, pixels
        // discarded); then capture the final frame.
        let n = frames.max(1);
        for _ in 1..n {
            self.time += scenes::FALLBACK_DT;
            self.step_offscreen(frame, &view, width, height, scenes::FALLBACK_DT);
        }
        self.time += scenes::FALLBACK_DT;

        let (buffer, padded_bpr) = capture::create_readback(&self.ctx.device, width, height);
        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("lmv-capture-preset"),
            });
        capture::record_clear(&mut encoder, &view);
        let _ = self.draw_frame(
            frame,
            &mut encoder,
            &view,
            width,
            height,
            scenes::FALLBACK_DT,
        );
        capture::record_copy(&mut encoder, &texture, &buffer, padded_bpr, width, height);
        self.ctx.queue.submit(std::iter::once(encoder.finish()));

        #[cfg(feature = "text")]
        self.text_layer.end_frame();

        capture::read_back(&self.ctx.device, &buffer, width, height, padded_bpr)
    }

    /// Drive preset `name` with **real audio through the real analyzer** and
    /// capture the frames at `at_frames` (Plan 0013). The PCM is fed hop-by-hop
    /// into a fresh [`Analyzer`](crate::dsp::Analyzer) (format validated at the
    /// intake boundary — the source-agnostic rule); each produced
    /// [`AnalysisFrame`] drives one rendered frame, so `at_frames` indexes the
    /// hop sequence (frame 0 is the first hop). Deterministic: scenes are rebuilt
    /// to their seed and the clock resets to 0, exactly like [`capture_preset`].
    ///
    /// This is in-memory PCM only — no file, decoder, or OS audio-source code,
    /// just like a frontend pushing samples. Returned images are in `at_frames`
    /// order; an index past the audio length is an error.
    pub fn capture_audio(
        &mut self,
        name: &str,
        pcm: &[f32],
        format: AudioFormat,
        at_frames: &[u32],
    ) -> Result<Vec<CaptureImage>, RenderError> {
        if !self.select_preset_by_name(name) {
            return Err(RenderError::UnknownPreset(name.to_string()));
        }
        let mut analyzer = crate::dsp::Analyzer::new(format).map_err(RenderError::AudioFormat)?;

        self.scenes = scenes::create_all(&self.ctx.device, self.ctx.surface_format());
        self.background.reset_resources();
        self.time = 0.0;
        self.configure_active_scene();

        let (width, height) = (self.ctx.config.width, self.ctx.config.height);
        let target_format = self.ctx.surface_format();
        let (texture, view) =
            capture::create_target(&self.ctx.device, target_format, width, height);

        let hop_samples = crate::dsp::HOP_SIZE * format.channels as usize;
        let mut captured: Vec<(u32, CaptureImage)> = Vec::with_capacity(at_frames.len());

        for (index, hop) in pcm.chunks(hop_samples).enumerate() {
            let frame_index = index as u32;
            analyzer.push_interleaved(hop);
            let analysis = analyzer.take_frame();
            self.time += scenes::FALLBACK_DT;

            let wanted = at_frames.contains(&frame_index)
                && !captured.iter().any(|(i, _)| *i == frame_index);
            if wanted {
                let (buffer, padded_bpr) =
                    capture::create_readback(&self.ctx.device, width, height);
                let mut encoder =
                    self.ctx
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("lmv-capture-audio"),
                        });
                capture::record_clear(&mut encoder, &view);
                let _ = self.draw_frame(
                    &analysis,
                    &mut encoder,
                    &view,
                    width,
                    height,
                    scenes::FALLBACK_DT,
                );
                capture::record_copy(&mut encoder, &texture, &buffer, padded_bpr, width, height);
                self.ctx.queue.submit(std::iter::once(encoder.finish()));
                #[cfg(feature = "text")]
                self.text_layer.end_frame();
                let img = capture::read_back(&self.ctx.device, &buffer, width, height, padded_bpr)?;
                captured.push((frame_index, img));
            } else {
                self.step_offscreen(&analysis, &view, width, height, scenes::FALLBACK_DT);
            }
        }

        at_frames
            .iter()
            .map(|idx| {
                captured
                    .iter()
                    .find(|(i, _)| i == idx)
                    .map(|(_, img)| img.clone())
                    .ok_or(RenderError::CaptureReadback)
            })
            .collect()
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
        dt: f32,
    ) {
        let mut encoder = self
            .ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("lmv-capture-step"),
            });
        capture::record_clear(&mut encoder, view);
        let _ = self.draw_frame(frame, &mut encoder, view, width, height, dt);
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
    // Plan 0008 Phase 2 names. Test asserts use `expect`/`panic!`, allowed here
    // over the file's hot-path panic-denial pragma — test code is not the render
    // path (`headless_or_skip` panics on an unexpected build error).
    #![allow(clippy::expect_used, clippy::indexing_slicing, clippy::panic)]

    use super::{HeadlessOptions, RenderError, Renderer, Roster};
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

    /// Build a headless `Renderer`, or return `None` (a logged skip) when the
    /// runner exposes no usable GPU adapter (ADR-0016). A missing adapter is an
    /// environmental property of the CI runner — macOS has no software Metal
    /// fallback — not a code failure, so the GPU-capture tests skip on it rather
    /// than panic; any *other* build error still panics loudly. On Windows WARP
    /// an adapter is always present, so the callers' assertions run in full.
    fn headless_or_skip(opts: HeadlessOptions) -> Option<Renderer> {
        match Renderer::new_headless(opts) {
            Ok(r) => Some(r),
            Err(RenderError::RequestAdapter(_)) => {
                eprintln!("skipped: no GPU adapter on this runner (ADR-0016)");
                None
            }
            Err(e) => panic!("headless renderer build failed: {e}"),
        }
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
        let Some(mut renderer) = headless_or_skip(HeadlessOptions {
            width: 256,
            height: 256,
            prefer_software: true,
        }) else {
            return;
        };

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
        let Some(mut renderer) = headless_or_skip(HeadlessOptions {
            width: 128,
            height: 128,
            prefer_software: true,
        }) else {
            return;
        };
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

    /// Plan 0010 review finding #1: a line generator that hits the segment cap
    /// must **surface** the truncation, never cut silently (ADR-0007). An
    /// L-system whose depth blows past the cap reports a `CapOverflow` through
    /// `configure`, read back via `cap_overflow()`; a grammar that fits reports
    /// `None`. This is the surfacing half of the cap contract the mechanism
    /// tracked but nothing exercised.
    #[test]
    fn oversized_lsystem_surfaces_a_cap_overflow() {
        let Some(mut renderer) = headless_or_skip(HeadlessOptions {
            width: 64,
            height: 64,
            prefer_software: true,
        }) else {
            return;
        };

        // F -> ten F's per iteration: depth 5 is 100k draw steps, far past the
        // 20k cap, so the build truncates and must report the drop.
        let huge = Preset::from_toml_str(
            "system = \"lsystem\"\nname = \"Huge\"\n\
             [generator]\naxiom = \"F\"\nrules = { F = \"FFFFFFFFFF\" }\n\
             angle_deg = 20\nmax_depth = 5\n",
        )
        .expect("valid lsystem preset");
        renderer.set_presets(vec![huge]);
        let overflow = renderer
            .cap_overflow()
            .expect("an oversized L-system surfaces its cap truncation");
        assert!(
            overflow.dropped > 0,
            "the dropped-segment count is reported"
        );

        // A modest grammar (F -> FF, depth 5 = 32 segments) fits — no overflow.
        let small = Preset::from_toml_str(
            "system = \"lsystem\"\nname = \"Small\"\n\
             [generator]\naxiom = \"F\"\nrules = { F = \"FF\" }\n\
             angle_deg = 20\nmax_depth = 5\n",
        )
        .expect("valid lsystem preset");
        renderer.set_presets(vec![small]);
        assert!(
            renderer.cap_overflow().is_none(),
            "a grammar that fits within the cap reports no overflow"
        );
    }

    /// Plan 0018 Phase 4: the per-frame geometry mirror must also surface a cap
    /// truncation through `cap_overflow()`, reusing the ADR-0007 `CapOverflow`
    /// path — never a silent cut. A dense rose replicated six-fold blows past the
    /// 20k cap; a modest one fits. Unlike the L-system's load-time overflow, this
    /// one is computed per frame, so it surfaces only after a frame has rendered.
    #[test]
    fn oversized_mirror_surfaces_a_cap_overflow() {
        let Some(mut renderer) = headless_or_skip(HeadlessOptions {
            width: 64,
            height: 64,
            prefer_software: true,
        }) else {
            return;
        };
        let frame = AnalysisFrame::default();

        // ~5000 chords replicated six-fold = ~30k segments, far past the 20k cap.
        let huge = Preset::from_toml_str(
            "system = \"parametric_curve\"\nname = \"MirrorHuge\"\n\
             [curve]\nfamily = \"maurer_rose\"\n\
             [params]\nsamples = \"5000\"\nmirror_order = \"6\"\n",
        )
        .expect("valid parametric preset");
        renderer.set_presets(vec![huge]);
        // Render frames so the per-frame mirror replication runs and records the drop.
        renderer
            .capture_preset("MirrorHuge", &frame, 2)
            .expect("capture MirrorHuge");
        let overflow = renderer
            .cap_overflow()
            .expect("an oversized mirror surfaces its cap truncation");
        assert!(
            overflow.dropped > 0,
            "the dropped-segment count is reported"
        );

        // A modest rose at order 3 stays well under the cap — no overflow.
        let small = Preset::from_toml_str(
            "system = \"parametric_curve\"\nname = \"MirrorSmall\"\n\
             [curve]\nfamily = \"maurer_rose\"\n\
             [params]\nsamples = \"200\"\nmirror_order = \"3\"\n",
        )
        .expect("valid parametric preset");
        renderer.set_presets(vec![small]);
        renderer
            .capture_preset("MirrorSmall", &frame, 2)
            .expect("capture MirrorSmall");
        assert!(
            renderer.cap_overflow().is_none(),
            "a mirror that fits within the cap reports no overflow"
        );
    }
}
