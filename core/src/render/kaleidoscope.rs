//! Screen-space kaleidoscope (ADR-0018 composite stage 4, Plan 0018 Phase 7): a
//! post-pass that folds the composited frame into `N` mirrored wedges before
//! present — the general, engine-wide kaleidoscope (distinct from the line-only
//! *geometry* mirror of Phase 4, which replicates segments; this folds pixels).
//!
//! The fold is dihedral: each output pixel's angle is wrapped into one
//! `2*pi/order` wedge and mirrored within it, so the frame is invariant under a
//! `2*pi/order` rotation and carries a mirror line per wedge. `kaleido_angle`
//! rotates the whole fold. Driven by the `kaleido_order` / `kaleido_angle` named
//! params.
//!
//! **Identity passthrough when `kaleido_order < 2`** — every shipped preset until
//! one opts in — so the renderer skips this stage entirely: no offscreen, no
//! pipeline, golden/determinism unchanged, the NFR §1 iGPU floor pays nothing,
//! and (like the background/trails passes) the DX12 WARP software adapter never
//! sees a coexisting fold pipeline during the no-kaleidoscope captures. When
//! active the pipeline builds lazily and is dropped on the capture scene-rebuild.
//!
//! Runs at a fixed 16:9 internal resolution, presented stretched to the surface —
//! the same resolution-independent approach the other post/present passes take
//! (correct on a 16:9 display; distorted otherwise — a documented v1 limitation).

// Hot-path panic-denial pragma (Plan 0002 Phase 2; render/ is scanned by the
// hygiene guard). The fold pass encodes every displayed frame it is active.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

/// Fixed internal resolution (16:9), presented stretched (module docs); the fold
/// is aspect-corrected to this ratio so the wedges are symmetric on a 16:9 frame.
const KALEIDO_W: u32 = 1280;
const KALEIDO_H: u32 = 720;

/// `kaleido_order` default — 1 = identity, so an unbound preset is unaffected.
const DEFAULT_ORDER: f32 = 1.0;
/// `kaleido_angle` default — no rotation.
const DEFAULT_ANGLE: f32 = 0.0;

/// Below this order the fold is the identity passthrough (the stage is skipped).
const MIN_ACTIVE_ORDER: f32 = 2.0;
/// Ceiling on the fold order — beyond a couple dozen wedges the fold is a blur.
const MAX_ORDER: f32 = 48.0;

const SHADER: &str = r#"
struct K { v: vec4<f32> } // x: order, y: angle, z: aspect

@group(0) @binding(0) var<uniform> u: K;
@group(0) @binding(1) var t_src: texture_2d<f32>;
@group(0) @binding(2) var samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var pts = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0),
    );
    let p = pts[vi];
    var out: VsOut;
    out.pos = vec4<f32>(p, 0.0, 1.0);
    out.uv = vec2<f32>(0.5 * p.x + 0.5, 0.5 - 0.5 * p.y);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let order = max(u.v.x, 1.0);
    let angle = u.v.y;
    let aspect = max(u.v.z, 0.001);

    // Centre and aspect-correct so the wedges are radially symmetric.
    var p = in.uv - vec2<f32>(0.5, 0.5);
    p.x = p.x * aspect;

    let r = length(p);
    let seg = 6.28318530 / order;
    var a = atan2(p.y, p.x) + angle;
    // Wrap into one wedge, then mirror within it (dihedral fold).
    a = a - seg * floor(a / seg);
    a = abs(a - seg * 0.5);

    // Reconstruct the sample coordinate from the folded angle + original radius.
    var q = vec2<f32>(cos(a), sin(a)) * r;
    q.x = q.x / aspect;
    let s_uv = q + vec2<f32>(0.5, 0.5);
    return vec4<f32>(textureSample(t_src, samp, s_uv).rgb, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct K {
    v: [f32; 4],
}

struct Resources {
    // The offscreen the composite (background + scene [+ trails]) renders into.
    // Kept alive so `src_view` stays valid; not read after construction.
    _src: wgpu::Texture,
    src_view: wgpu::TextureView,
    uniform: wgpu::Buffer,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
}

impl Resources {
    fn build(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let src = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("kaleido-src"),
            size: wgpu::Extent3d {
                width: KALEIDO_W,
                height: KALEIDO_H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: surface_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let src_view = src.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("kaleido-sampler"),
            // Clamp so folded coords past the edge sample the border, not wrap.
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("kaleido-uniform"),
            size: std::mem::size_of::<K>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("kaleido-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("kaleido-bind-layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("kaleido-bind-group"),
            layout: &bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&src_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("kaleido-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("kaleido-pipeline"),
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
            _src: src,
            src_view,
            uniform,
            pipeline,
            bind_group,
        }
    }
}

/// The engine screen-space kaleidoscope stage. Not a [`Scene`](super::scenes::Scene):
/// it is driven by the `kaleido_*` named params the renderer routes to it, and it
/// folds the composited frame before present (ADR-0018).
pub struct Kaleidoscope {
    device: wgpu::Device,
    surface_format: wgpu::TextureFormat,
    res: Option<Resources>,
    order: f32,
    angle: f32,
}

impl Kaleidoscope {
    /// Store the device/format for a lazy build; no GPU resources yet.
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        Self {
            device: device.clone(),
            surface_format,
            res: None,
            order: DEFAULT_ORDER,
            angle: DEFAULT_ANGLE,
        }
    }

    /// Reset the fold params to their defaults (each frame, before routing).
    pub fn reset_params(&mut self) {
        self.order = DEFAULT_ORDER;
        self.angle = DEFAULT_ANGLE;
    }

    /// Apply one named parameter, returning whether it was a `kaleido_*` param.
    pub fn set_param(&mut self, name: &str, value: f32) -> bool {
        match name {
            "kaleido_order" => self.order = value,
            "kaleido_angle" => self.angle = value,
            _ => return false,
        }
        true
    }

    /// Drop the lazily-built resources — used on the capture scene-rebuild so a
    /// stale fold pipeline never lingers to mis-render the next capture's scene on
    /// the WARP adapter (module docs).
    pub fn reset_resources(&mut self) {
        self.res = None;
    }

    /// Whether the fold is active this frame (order at least 2; below that it is
    /// the identity passthrough).
    pub fn active(&self) -> bool {
        self.order >= MIN_ACTIVE_ORDER && self.order.is_finite()
    }

    /// The aspect the composite should render at into the fold's input — the fixed
    /// internal 16:9, presented stretched.
    pub fn aspect() -> f32 {
        KALEIDO_W as f32 / KALEIDO_H as f32
    }

    /// Build the resources if needed and return the offscreen view the composite
    /// (background + scene, or the trails output) renders into this frame. `None`
    /// only if the resources are absent (never, after the build) — the caller
    /// falls back to the surface. Called when [`active`](Self::active).
    pub fn begin(&mut self, _encoder: &mut wgpu::CommandEncoder) -> Option<&wgpu::TextureView> {
        if self.res.is_none() {
            self.res = Some(Resources::build(&self.device, self.surface_format));
        }
        self.res.as_ref().map(|res| &res.src_view)
    }

    /// Fold the input offscreen into `surface_view`. Called after the composite
    /// has rendered into the [`begin`](Self::begin) target, when
    /// [`active`](Self::active).
    pub fn resolve(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        surface_view: &wgpu::TextureView,
    ) {
        let Some(res) = self.res.as_ref() else {
            return;
        };
        let order = self.order.clamp(MIN_ACTIVE_ORDER, MAX_ORDER);
        queue.write_buffer(
            &res.uniform,
            0,
            bytemuck::bytes_of(&K {
                v: [order, self.angle, Self::aspect(), 0.0],
            }),
        );
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("kaleido-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: surface_view,
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
        pass.set_pipeline(&res.pipeline);
        pass.set_bind_group(0, &res.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
