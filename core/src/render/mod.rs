//! The render seam: take an [`AnalysisFrame`], draw a scene.
//!
//! The render loop is driven by the frontend at display cadence and is fully
//! decoupled from audio delivery — the ring buffer is the seam (CLAUDE.md).

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

use crate::dsp::AnalysisFrame;
use crate::scenes::Scene;
pub use context::{RenderContext, RenderError};

/// Owns the GPU context plus the scene roster and renders one frame per call.
pub struct Renderer {
    ctx: RenderContext,
    scenes: Vec<Box<dyn Scene>>,
    active: usize,
}

impl Renderer {
    /// Build a renderer drawing into `target` (a safe window handle — the
    /// standalone path).
    pub fn new(
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        width: u32,
        height: u32,
    ) -> Result<Self, RenderError> {
        let ctx = RenderContext::new(target, width, height)?;
        let scenes = crate::scenes::create_all(&ctx.device, ctx.surface_format());
        Ok(Self {
            ctx,
            scenes,
            active: 0,
        })
    }

    /// Renderer targeting a native Win32 window the host owns — the C ABI
    /// path (foobar2000 shim).
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
        let scenes = crate::scenes::create_all(&ctx.device, ctx.surface_format());
        Ok(Self {
            ctx,
            scenes,
            active: 0,
        })
    }

    /// Reconfigure the surface for a new window size.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.ctx.resize(width, height);
    }

    /// Switch to the next scene; returns its name. Instant — every scene is
    /// built at startup, so cycling never hitches a live show.
    #[allow(
        clippy::indexing_slicing,
        reason = "active is kept < scenes.len() (modulo len), a valid index into the non-empty roster"
    )]
    pub fn cycle_scene(&mut self) -> &'static str {
        self.active = (self.active + 1) % self.scenes.len();
        self.scenes[self.active].name()
    }

    /// Name of the currently active scene.
    #[allow(
        clippy::indexing_slicing,
        reason = "active is kept < scenes.len(), a valid index into the non-empty roster"
    )]
    pub fn scene_name(&self) -> &'static str {
        self.scenes[self.active].name()
    }

    /// Draw the current scene for this analysis frame. Lost/outdated surfaces
    /// self-heal by reconfiguring; timeouts/occlusion skip the frame; only a
    /// validation failure (a bug) bubbles up.
    #[allow(
        clippy::indexing_slicing,
        reason = "active is kept < scenes.len(), a valid index into the non-empty roster"
    )]
    pub fn render(&mut self, frame: &AnalysisFrame) -> Result<(), RenderError> {
        self.scenes[self.active].update(frame);
        let Some(surface_tex) = self.acquire()? else {
            return Ok(()); // transient (timeout/occluded) — skip this frame
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

        let aspect = self.ctx.config.width as f32 / self.ctx.config.height.max(1) as f32;
        self.scenes[self.active].render(&self.ctx.queue, &mut encoder, &view, aspect);

        self.ctx.queue.submit(std::iter::once(encoder.finish()));
        self.ctx.queue.present(surface_tex);
        Ok(())
    }

    fn acquire(&self) -> Result<Option<wgpu::SurfaceTexture>, RenderError> {
        use wgpu::CurrentSurfaceTexture as C;
        match self.ctx.surface.get_current_texture() {
            C::Success(t) | C::Suboptimal(t) => Ok(Some(t)),
            C::Timeout | C::Occluded => Ok(None),
            C::Outdated | C::Lost => {
                self.ctx.reconfigure();
                match self.ctx.surface.get_current_texture() {
                    C::Success(t) | C::Suboptimal(t) => Ok(Some(t)),
                    C::Validation => Err(RenderError::SurfaceValidation),
                    _ => Ok(None),
                }
            }
            C::Validation => Err(RenderError::SurfaceValidation),
        }
    }
}
