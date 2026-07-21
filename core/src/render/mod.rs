//! The render seam: take an [`AnalysisFrame`], draw a scene.
//!
//! The render loop is driven by the frontend at display cadence and is fully
//! decoupled from audio delivery — the ring buffer is the seam (CLAUDE.md).

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

    pub fn resize(&mut self, width: u32, height: u32) {
        self.ctx.resize(width, height);
    }

    /// Switch to the next scene; returns its name. Instant — every scene is
    /// built at startup, so cycling never hitches a live show.
    pub fn cycle_scene(&mut self) -> &'static str {
        self.active = (self.active + 1) % self.scenes.len();
        self.scenes[self.active].name()
    }

    pub fn scene_name(&self) -> &'static str {
        self.scenes[self.active].name()
    }

    /// Draw the current scene for this analysis frame. Lost/outdated surfaces
    /// self-heal by reconfiguring; timeouts/occlusion skip the frame; only a
    /// validation failure (a bug) bubbles up.
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
