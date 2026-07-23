//! Reusable ping-pong offscreen field for stateful feedback scenes (ADR-0012).
//!
//! Two same-format textures a simulation swaps each sub-step: the sim samples
//! the previous state (the *read* view) and writes the next (the *write* view),
//! then the field swaps so the fresh state becomes the next read. This is the
//! engine's first feedback path; the reaction-diffusion scene is its first user
//! and future warp/feedback variants reuse it (ADR-0002 named it a deferred
//! follow-up).
//!
//! **Composition, not engine machinery.** The field owns only the texture pair
//! and the read/write selector; the *scene* owns its sim/present pipelines and
//! the shader that steps the field. That keeps the `Scene` seam thin — ADR-0012
//! rejected an engine-managed multi-pass pipeline for exactly this reason.

// Hot-path panic-denial pragma (Plan 0002 Phase 2; render/ is scanned by the
// hygiene guard). A feedback scene encodes its passes every displayed frame.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

/// Two offscreen textures a feedback scene ping-pongs between. Held by named
/// fields (not a `[_; 2]`) so read/write selection needs no array indexing on
/// the hot path.
pub(crate) struct PingPongField {
    // Kept alive so the views stay valid; not read after construction.
    _tex_a: wgpu::Texture,
    _tex_b: wgpu::Texture,
    view_a: wgpu::TextureView,
    view_b: wgpu::TextureView,
    /// `true`: read from A, write to B. `swap` flips it each sub-step.
    reading_a: bool,
}

impl PingPongField {
    /// The field's texel format. `Rgba16Float` is renderable and filterable on
    /// the wgpu targets we ship (DX12/Vulkan/Metal) — the R/G channels hold the
    /// two Gray-Scott species with the headroom the slow gradients need
    /// (ADR-0012 Risks: the `Rgba8Unorm` fallback would band).
    pub(crate) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

    /// Allocate the texture pair at a fixed internal `width`×`height` grid,
    /// decoupled from the surface size (ADR-0012: the simulation is
    /// resolution-independent). Contents are undefined until the scene's seed
    /// pass writes every texel before the first sub-step reads it.
    pub(crate) fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let make = |label: &str| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: Self::FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
        };
        let tex_a = make("lmv-ppf-a");
        let tex_b = make("lmv-ppf-b");
        let view_a = tex_a.create_view(&wgpu::TextureViewDescriptor::default());
        let view_b = tex_b.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            _tex_a: tex_a,
            _tex_b: tex_b,
            view_a,
            view_b,
            reading_a: true,
        }
    }

    /// Texture A's view — a scene binds it once to build its A-read bind group.
    pub(crate) fn view_a(&self) -> &wgpu::TextureView {
        &self.view_a
    }

    /// Texture B's view — a scene binds it once to build its B-read bind group.
    pub(crate) fn view_b(&self) -> &wgpu::TextureView {
        &self.view_b
    }

    /// Whether A is the current read source (so the scene picks the matching
    /// pre-built bind group without rebuilding one each sub-step).
    pub(crate) fn reading_a(&self) -> bool {
        self.reading_a
    }

    /// The view the next sub-step (or the present pass) samples from.
    pub(crate) fn read_view(&self) -> &wgpu::TextureView {
        if self.reading_a {
            &self.view_a
        } else {
            &self.view_b
        }
    }

    /// The view the next sub-step renders into.
    pub(crate) fn write_view(&self) -> &wgpu::TextureView {
        if self.reading_a {
            &self.view_b
        } else {
            &self.view_a
        }
    }

    /// Flip read and write — call once after each sub-step's render pass.
    pub(crate) fn swap(&mut self) {
        self.reading_a = !self.reading_a;
    }
}
