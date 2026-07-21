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

pub mod context;
pub mod scenes;

use crate::dsp::AnalysisFrame;
use crate::preset::{Preset, SystemKind, Variables};
pub use context::{RenderContext, RenderError};
use scenes::Scene;

/// A preset's system to its slot in the roster built by [`scenes::create_all`].
/// The legacy scenes occupy later slots but no preset addresses them.
fn system_slot(system: SystemKind) -> usize {
    match system {
        SystemKind::FragmentField => 0,
        SystemKind::Swarm => 1,
    }
}

/// Owns the GPU context, the built-in systems, and the loaded presets; renders
/// one frame per call by evaluating the active preset into the active system.
pub struct Renderer {
    ctx: RenderContext,
    scenes: Vec<Box<dyn Scene>>,
    presets: Vec<Preset>,
    active: usize,
    /// Shared scene clock (seconds), advanced one fixed step per rendered frame.
    /// The single source for both an expression's `time` and system animation.
    time: f32,
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
        Ok(Self {
            ctx,
            scenes,
            presets: crate::preset::default_presets(),
            active: 0,
            time: 0.0,
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
        Ok(Self {
            ctx,
            scenes,
            presets: crate::preset::default_presets(),
            active: 0,
            time: 0.0,
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
        if presets.is_empty() {
            return;
        }
        self.presets = presets;
        if self.active >= self.presets.len() {
            self.active = 0;
        }
    }

    /// Switch to the next preset; returns its name. Instant — every system is
    /// built at startup, so cycling never hitches a live show.
    pub fn cycle_preset(&mut self) -> &str {
        if !self.presets.is_empty() {
            self.active = (self.active + 1) % self.presets.len();
        }
        self.preset_name()
    }

    /// Name of the currently active preset.
    pub fn preset_name(&self) -> &str {
        self.presets
            .get(self.active)
            .map(|p| p.name.as_str())
            .unwrap_or("no presets")
    }

    /// Name of the built-in system the active preset drives (e.g. the frontend
    /// shows it next to the preset name).
    pub fn active_system_name(&self) -> &'static str {
        self.presets
            .get(self.active)
            .and_then(|p| self.scenes.get(system_slot(p.system)))
            .map(|scene| scene.name())
            .unwrap_or("")
    }

    /// Draw the current preset for this analysis frame. Lost/outdated surfaces
    /// self-heal by reconfiguring; timeouts/occlusion skip the frame; only a
    /// validation failure (a bug) bubbles up.
    pub fn render(&mut self, frame: &AnalysisFrame) -> Result<(), RenderError> {
        self.time += scenes::SCENE_DT;

        let Self {
            ctx,
            scenes,
            presets,
            active,
            time,
        } = self;

        let Some(preset) = presets.get(*active) else {
            return Ok(()); // no presets loaded — nothing to draw
        };
        let Some(scene) = scenes.get_mut(system_slot(preset.system)) else {
            return Ok(());
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

        let Some(surface_tex) = Self::acquire(ctx)? else {
            return Ok(()); // transient (timeout/occluded) — skip this frame
        };
        let view = surface_tex
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("lmv-frame"),
            });

        let aspect = ctx.config.width as f32 / ctx.config.height.max(1) as f32;
        scene.render(&ctx.queue, &mut encoder, &view, aspect);

        ctx.queue.submit(std::iter::once(encoder.finish()));
        ctx.queue.present(surface_tex);
        Ok(())
    }

    fn acquire(ctx: &RenderContext) -> Result<Option<wgpu::SurfaceTexture>, RenderError> {
        use wgpu::CurrentSurfaceTexture as C;
        match ctx.surface.get_current_texture() {
            C::Success(t) | C::Suboptimal(t) => Ok(Some(t)),
            C::Timeout | C::Occluded => Ok(None),
            C::Outdated | C::Lost => {
                ctx.reconfigure();
                match ctx.surface.get_current_texture() {
                    C::Success(t) | C::Suboptimal(t) => Ok(Some(t)),
                    C::Validation => Err(RenderError::SurfaceValidation),
                    _ => Ok(None),
                }
            }
            C::Validation => Err(RenderError::SurfaceValidation),
        }
    }
}
