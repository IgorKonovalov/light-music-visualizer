//! Feedback trails (ADR-0018 composite stage 3, Plan 0018 Phase 6): route the
//! composited frame (background + scene) through a fade-and-accumulate feedback
//! so moving shapes leave light trails. Reuses Plan 0014's
//! [`PingPongField`](super::feedback::PingPongField) for the accumulation — no
//! second feedback mechanism.
//!
//! The blend is a **max-decay**: `accum = max(current, fade * previous)`. The
//! current frame shows at full brightness while past frames fade at `fade` per
//! frame, so a bright additive stroke leaves a crisp head and a fading tail; a
//! static (dark) backdrop is stable (its own max), so nothing blows up. `fade`
//! comes from the `trails` named param (0 = off).
//!
//! **Off by default (passthrough).** When `trails <= 0` — every shipped preset
//! until one opts in — the renderer skips this stage entirely: no offscreen
//! target, no pipelines, so golden/determinism are unchanged, the NFR §1 iGPU
//! floor pays nothing, and (like the background pass) the DX12 WARP software
//! adapter never sees a coexisting feedback pipeline during the no-trails
//! captures. When active, the pipelines build lazily and the accumulation is
//! **reset on the capture scene-rebuild**, so a headless capture stays a pure
//! function of its inputs (NFR §6).
//!
//! The composite runs at a fixed 16:9 internal resolution, presented stretched to
//! the surface — the same resolution-independent approach the reaction-diffusion
//! and attractor presents already take (correct on a 16:9 display; distorted
//! otherwise — a documented v1 limitation).

// Hot-path panic-denial pragma (Plan 0002 Phase 2; render/ is scanned by the
// hygiene guard). The trails stage encodes its passes every displayed frame.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

use super::feedback::PingPongField;

/// Fixed internal composite resolution (16:9), presented stretched to the surface
/// (module docs). High enough that line trails stay crisp, cheap enough for the
/// iGPU floor when a trails preset is active.
const TRAILS_W: u32 = 1280;
const TRAILS_H: u32 = 720;

/// `trails` param default — off, so an unbound preset pays nothing.
const DEFAULT_TRAILS: f32 = 0.0;

/// Hard ceiling on the decay factor: `1.0` would never fade (an ever-brightening
/// smear), so keep it strictly below.
const MAX_FADE: f32 = 0.98;

const TRAILS_SHADER: &str = r#"
struct Fade { v: vec4<f32> } // x: fade factor

@group(0) @binding(0) var<uniform> u: Fade;
@group(0) @binding(1) var t_composited: texture_2d<f32>;
@group(0) @binding(2) var t_accum: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    // Fullscreen triangle; map clip space to [0,1] uv (y flipped for texture space).
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
    let cur = textureSample(t_composited, samp, in.uv).rgb;
    let prev = textureSample(t_accum, samp, in.uv).rgb;
    // Max-decay: the current frame at full brightness, the past fading by `fade`.
    return vec4<f32>(max(cur, prev * u.v.x), 1.0);
}
"#;

const PRESENT_SHADER: &str = r#"
@group(0) @binding(0) var t_accum: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

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
    return vec4<f32>(textureSample(t_accum, samp, in.uv).rgb, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Fade {
    v: [f32; 4],
}

/// The trails GPU resources, built lazily on the first active frame.
struct Resources {
    // The offscreen the composite (background + scene) renders into each frame.
    // Kept alive so `composited_view` stays valid; not read after construction.
    _composited: wgpu::Texture,
    composited_view: wgpu::TextureView,
    accum: PingPongField,
    fade_uniform: wgpu::Buffer,
    trails_pipeline: wgpu::RenderPipeline,
    // One bind group per accumulation read-side (composited + accum read + fade + sampler).
    trails_bg_a: wgpu::BindGroup,
    trails_bg_b: wgpu::BindGroup,
    present_pipeline: wgpu::RenderPipeline,
    present_bg_a: wgpu::BindGroup,
    present_bg_b: wgpu::BindGroup,
}

impl Resources {
    fn build(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let composited = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("trails-composited"),
            size: wgpu::Extent3d {
                width: TRAILS_W,
                height: TRAILS_H,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: surface_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let composited_view = composited.create_view(&wgpu::TextureViewDescriptor::default());
        let accum = PingPongField::new(device, TRAILS_W, TRAILS_H);

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("trails-sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let fade_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("trails-fade"),
            size: std::mem::size_of::<Fade>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let trails_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("trails-shader"),
            source: wgpu::ShaderSource::Wgsl(TRAILS_SHADER.into()),
        });
        let trails_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("trails-bind-layout"),
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
                tex_entry(1),
                tex_entry(2),
                samp_entry(3),
            ],
        });
        let make_trails_bg = |accum_view: &wgpu::TextureView, label: &str| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &trails_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: fade_uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&composited_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(accum_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            })
        };
        let trails_bg_a = make_trails_bg(accum.view_a(), "trails-bg-a");
        let trails_bg_b = make_trails_bg(accum.view_b(), "trails-bg-b");
        let trails_pipeline = fullscreen_pipeline(
            device,
            &trails_shader,
            &trails_layout,
            PingPongField::FORMAT,
            "trails",
        );

        let present_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("trails-present-shader"),
            source: wgpu::ShaderSource::Wgsl(PRESENT_SHADER.into()),
        });
        let present_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("trails-present-bind-layout"),
            entries: &[tex_entry(0), samp_entry(1)],
        });
        let make_present_bg = |accum_view: &wgpu::TextureView, label: &str| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &present_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(accum_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            })
        };
        let present_bg_a = make_present_bg(accum.view_a(), "trails-present-bg-a");
        let present_bg_b = make_present_bg(accum.view_b(), "trails-present-bg-b");
        let present_pipeline = fullscreen_pipeline(
            device,
            &present_shader,
            &present_layout,
            surface_format,
            "trails-present",
        );

        Self {
            _composited: composited,
            composited_view,
            accum,
            fade_uniform,
            trails_pipeline,
            trails_bg_a,
            trails_bg_b,
            present_pipeline,
            present_bg_a,
            present_bg_b,
        }
    }

    /// Clear both accumulation textures so the first feedback frame reads a
    /// defined (black) trail rather than undefined texels.
    fn clear_accum(&self, encoder: &mut wgpu::CommandEncoder) {
        for view in [self.accum.view_a(), self.accum.view_b()] {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("trails-clear"),
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
        }
    }
}

fn tex_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

fn samp_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

fn fullscreen_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    bind_layout: &wgpu::BindGroupLayout,
    target_format: wgpu::TextureFormat,
    label: &str,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(&format!("{label}-pipeline-layout")),
        bind_group_layouts: &[Some(bind_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(&format!("{label}-pipeline")),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: target_format,
                blend: Some(wgpu::BlendState::REPLACE),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// The engine feedback-trails stage. Not a [`Scene`](super::scenes::Scene): it is
/// driven by the `trails` named param the renderer routes to it, and it wraps the
/// composite (background + scene) in a fade-and-accumulate feedback (ADR-0018).
pub struct Trails {
    device: wgpu::Device,
    surface_format: wgpu::TextureFormat,
    res: Option<Resources>,
    amount: f32,
}

impl Trails {
    /// Store the device/format for a lazy build; no GPU resources yet.
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        Self {
            device: device.clone(),
            surface_format,
            res: None,
            amount: DEFAULT_TRAILS,
        }
    }

    /// Reset the `trails` amount to its default (each frame, before the active
    /// preset's bindings are routed).
    pub fn reset_params(&mut self) {
        self.amount = DEFAULT_TRAILS;
    }

    /// Apply one named parameter, returning whether it was the `trails` param.
    pub fn set_param(&mut self, name: &str, value: f32) -> bool {
        if name == "trails" {
            self.amount = value;
            true
        } else {
            false
        }
    }

    /// Drop the lazily-built resources so the accumulation restarts cleared — used
    /// on the capture scene-rebuild so a capture stays a pure function of its
    /// inputs, and so a stale trails pipeline never lingers to mis-render the next
    /// capture's scene on the WARP adapter (module docs).
    pub fn reset_resources(&mut self) {
        self.res = None;
    }

    /// Whether trails are active this frame (a preset bound `trails > 0`).
    pub fn active(&self) -> bool {
        self.amount > 0.0 && self.amount.is_finite()
    }

    /// The aspect the scene should render at into the composited target — the
    /// fixed internal 16:9, presented stretched.
    pub fn aspect() -> f32 {
        TRAILS_W as f32 / TRAILS_H as f32
    }

    /// Build the resources if needed (clearing the fresh accumulation) and return
    /// the offscreen view the background + scene render into this frame. Returns
    /// `None` only if the resources are absent (never, after the build above) —
    /// the caller falls back to the surface view. Called when
    /// [`active`](Self::active).
    pub fn begin(&mut self, encoder: &mut wgpu::CommandEncoder) -> Option<&wgpu::TextureView> {
        if self.res.is_none() {
            let res = Resources::build(&self.device, self.surface_format);
            res.clear_accum(encoder);
            self.res = Some(res);
        }
        self.res.as_ref().map(|res| &res.composited_view)
    }

    /// Fold this frame's composited target into the accumulation (max-decay) and
    /// present the result to `surface_view`. Called after the scene has rendered
    /// into the [`begin`](Self::begin) target, when [`active`](Self::active).
    pub fn resolve(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        surface_view: &wgpu::TextureView,
    ) {
        let Some(res) = self.res.as_mut() else {
            return;
        };
        let fade = self.amount.clamp(0.0, MAX_FADE);
        queue.write_buffer(
            &res.fade_uniform,
            0,
            bytemuck::bytes_of(&Fade {
                v: [fade, 0.0, 0.0, 0.0],
            }),
        );

        // Feedback pass: write the max-decay into the write side.
        let (trails_bg, write_view) = if res.accum.reading_a() {
            (&res.trails_bg_a, res.accum.view_b())
        } else {
            (&res.trails_bg_b, res.accum.view_a())
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("trails-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: write_view,
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
            pass.set_pipeline(&res.trails_pipeline);
            pass.set_bind_group(0, trails_bg, &[]);
            pass.draw(0..3, 0..1);
        }
        res.accum.swap();

        // Present the freshly-written accumulation to the surface.
        let present_bg = if res.accum.reading_a() {
            &res.present_bg_a
        } else {
            &res.present_bg_b
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("trails-present-pass"),
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
        pass.set_pipeline(&res.present_pipeline);
        pass.set_bind_group(0, present_bg, &[]);
        pass.draw(0..3, 0..1);
    }
}
