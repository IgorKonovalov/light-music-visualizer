//! The render seam: take an [`AnalysisFrame`], draw a scene.
//!
//! The render loop is driven by the frontend at display cadence and is fully
//! decoupled from audio delivery — the ring buffer is the seam (CLAUDE.md).

pub mod context;

use crate::dsp::AnalysisFrame;
use crate::scenes::spectrum::SpectrumScene;
pub use context::{RenderContext, RenderError};

/// Owns the GPU context plus the active scene and renders one frame per call.
pub struct Renderer {
    ctx: RenderContext,
    scene: SpectrumScene,
}

impl Renderer {
    pub fn new(
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        width: u32,
        height: u32,
    ) -> Result<Self, RenderError> {
        let ctx = RenderContext::new(target, width, height)?;
        let scene = SpectrumScene::new(&ctx.device, ctx.surface_format());
        Ok(Self { ctx, scene })
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.ctx.resize(width, height);
    }

    /// Draw the current scene for this analysis frame. Lost/outdated surfaces
    /// self-heal by reconfiguring; timeouts/occlusion skip the frame; only a
    /// validation failure (a bug) bubbles up.
    pub fn render(&mut self, frame: &AnalysisFrame) -> Result<(), RenderError> {
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

        self.scene
            .render(&self.ctx.queue, &mut encoder, &view, frame);

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
