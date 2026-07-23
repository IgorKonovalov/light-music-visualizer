//! Background pre-pass (ADR-0018): fills the whole frame with an audio-tintable
//! gradient + vignette *before* the active scene draws, so every scene composites
//! over a shared backdrop instead of clearing its own near-black. This pass
//! **owns the frame clear**; the scenes switched from `Clear` to `Load` (Plan
//! 0018 Phase 3), so a mid-composite pass never wipes what a prior stage drew.
//!
//! Driven by named params (`bg_hue`, `bg_bright`, `bg_vignette`) the renderer
//! routes here before the scene's own bindings. At the defaults (`bg_bright = 0`)
//! the backdrop is black, so a preset that binds none renders exactly as before —
//! the migration is neutral until a preset opts into a backdrop.
//!
//! **When no backdrop is bound (`bg_bright <= 0`) the pass is a plain black
//! clear** — no gradient pipeline is drawn, and the pipeline is not even built.
//! Two reasons: it is the NFR §1 passthrough win (an invisible black gradient
//! costs nothing), and — like the reaction-diffusion / attractor scenes' lazy
//! resources — it keeps a second fullscreen fragment pipeline off the device
//! during the headless no-bg captures, where the DX12 WARP software adapter would
//! otherwise mis-render the coexisting scene pipelines (a documented quirk with no
//! validation error; real hardware is unaffected).
//!
//! A fullscreen scene (fragment field, reaction-diffusion) draws opaquely over
//! the backdrop, so its bg params have no visible effect; the pass earns its keep
//! behind the *sparse* scenes (lines, swarm, attractor), where the empty space
//! between strokes and points reveals the gradient.

// Hot-path panic-denial pragma (Plan 0002 Phase 2, extended to render/ by the
// hygiene guard). Runs every displayed frame.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

/// Parameter defaults — a black backdrop when nothing is bound, so the composite
/// is byte-neutral against the pre-Phase-3 per-scene clears.
const DEFAULT_HUE: f32 = 0.0;
const DEFAULT_BRIGHT: f32 = 0.0;
const DEFAULT_VIGNETTE: f32 = 0.0;

const SHADER: &str = r#"
struct Bg {
    // x: hue, y: bright, z: vignette, w: unused
    v: vec4<f32>,
}

@group(0) @binding(0) var<uniform> u: Bg;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    // Single oversized triangle covers the viewport (no vertex buffer).
    var pts = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(pts[vi], 0.0, 1.0);
    out.ndc = pts[vi];
    return out;
}

// iq-style cosine palette, matching the scenes' colour language.
fn palette(t: f32) -> vec3<f32> {
    let d = vec3<f32>(0.10, 0.42, 0.62);
    return vec3<f32>(0.5) + vec3<f32>(0.5) * cos(6.28318 * (vec3<f32>(t) + d));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let hue = u.v.x;
    let bright = u.v.y;
    let vig_amt = u.v.z;

    // A gentle vertical gradient (a touch brighter toward the top) plus a radial
    // vignette that darkens the corners — the atmospheric backdrop.
    let grad = mix(0.72, 1.0, clamp(0.5 + 0.5 * in.ndc.y, 0.0, 1.0));
    let r = length(in.ndc);
    let vig = 1.0 - vig_amt * clamp(r * r, 0.0, 1.0);

    let col = palette(hue) * bright * grad * vig;
    return vec4<f32>(col, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Bg {
    v: [f32; 4],
}

/// The gradient pipeline + its uniform, built lazily on the first frame that
/// actually paints a backdrop (see the module docs on the WARP quirk).
struct Resources {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

impl Resources {
    fn build(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("background-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("background-params"),
            size: std::mem::size_of::<Bg>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("background-bind-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("background-bind-group"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("background-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("background-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    // Opaque: the backdrop establishes the frame the scene loads over.
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        Self {
            pipeline,
            uniforms,
            bind_group,
        }
    }
}

/// The engine-owned background pass. Not a [`Scene`](super::scenes::Scene): it is
/// driven by `bg_*` named params the renderer routes to it, and it runs before
/// the active scene in the fixed composite (ADR-0018). Its GPU pipeline is built
/// lazily on the first frame that paints a visible backdrop.
pub struct Background {
    device: wgpu::Device,
    surface_format: wgpu::TextureFormat,
    /// Gradient pipeline, built lazily (module docs: WARP + passthrough).
    res: Option<Resources>,
    hue: f32,
    bright: f32,
    vignette: f32,
}

impl Background {
    /// Store the device/format for a lazy pipeline build; no GPU resources yet.
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        Self {
            device: device.clone(),
            surface_format,
            res: None,
            hue: DEFAULT_HUE,
            bright: DEFAULT_BRIGHT,
            vignette: DEFAULT_VIGNETTE,
        }
    }

    /// Drop the lazily-built gradient pipeline so the next backdrop rebuilds it.
    /// Called when the renderer rebuilds its scenes for a capture (Plan 0013): a
    /// capture stays a pure function of its inputs, and — on the WARP software
    /// adapter — a bg preset's pipeline never lingers to mis-render the *next*
    /// capture's scene (module docs).
    pub fn reset_resources(&mut self) {
        self.res = None;
    }

    /// Reset every background param to its default (called each frame before the
    /// active preset's bindings are routed, so unbound params don't leak).
    pub fn reset_params(&mut self) {
        self.hue = DEFAULT_HUE;
        self.bright = DEFAULT_BRIGHT;
        self.vignette = DEFAULT_VIGNETTE;
    }

    /// Apply one named parameter, returning whether it was a background param
    /// (`bg_*`). The renderer routes to the scene only when this returns `false`,
    /// so scene and background param namespaces never collide.
    pub fn set_param(&mut self, name: &str, value: f32) -> bool {
        match name {
            "bg_hue" => self.hue = value,
            "bg_bright" => self.bright = value,
            "bg_vignette" => self.vignette = value,
            _ => return false,
        }
        true
    }

    /// Own the frame clear — the first pass of the composite. With no visible
    /// backdrop (`bg_bright <= 0`) this is a plain black clear (no pipeline); with
    /// one, it lazily builds the gradient pipeline and paints it fullscreen.
    pub fn render(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
    ) {
        if self.bright <= 0.0 {
            // Passthrough: a plain black clear establishes the frame without a
            // second fullscreen pipeline (module docs: NFR §1 + WARP).
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("background-clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            return;
        }

        let res = self
            .res
            .get_or_insert_with(|| Resources::build(&self.device, self.surface_format));
        queue.write_buffer(
            &res.uniforms,
            0,
            bytemuck::bytes_of(&Bg {
                v: [self.hue, self.bright, self.vignette, 0.0],
            }),
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("background-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    // The backdrop owns the clear: establish the frame here so no
                    // scene needs to (ADR-0018).
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&res.pipeline);
        pass.set_bind_group(0, &res.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
