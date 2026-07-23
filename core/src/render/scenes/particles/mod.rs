//! GPU compute-particle scenes: strange attractors (ADR-0015, Plan 0016). The
//! engine's **first compute pipeline** — a storage buffer of particles stepped
//! through an attractor map each frame by a compute shader, then drawn as
//! additive point-sprites with fading trails. This is idiom B of the four render
//! idioms; the CPU [`swarm`](super::swarm) is idiom B's ~10k CPU precursor,
//! replaced here by GPU-resident state that scales to 100k+ points with no CPU
//! round-trip.
//!
//! Trails reuse Plan 0014's [`PingPongField`](crate::render::feedback) rather
//! than a second feedback mechanism: each frame the previous accumulation texture
//! is drawn back faded (decay pass), the fresh points are added on top
//! (additive), and the result is composited to the surface (present pass). Trail
//! persistence is the named `fade` parameter; `fade = 0` clears the accumulation
//! each frame, reproducing the trail-free look.
//!
//! Every knob is an ADR-0002 layer-2 named parameter — the attractor
//! coefficients (`a`,`b`,`c`,`d`), look scalars (`size`,`hue`,`fade`), and a
//! beat-driven `reseed` — so a preset steers the cloud's shape and a beat
//! re-scatters it. All randomness is the seeded initial scatter (NFR 6): the
//! point cloud is a pure function of the seed and the fixed-`dt` step sequence,
//! so a capture reproduces bit-for-bit on one adapter.
//!
//! **GPU resources are built lazily, on first render** — the same discipline the
//! reaction-diffusion scene uses (see its module docs). `create_all` builds every
//! scene up front, but the compute pipeline + storage buffer + trail field are
//! constructed only when this scene is first drawn, so a capture that never
//! activates it never builds them (keeping the other scenes' WARP captures
//! unperturbed).
//!
//! The accumulation field is a fixed 16:9 offscreen, presented stretched to the
//! surface (aspect ignored, as the reaction-diffusion present does): correct on a
//! 16:9 display, uniformly stretched otherwise.

// Hot-path panic-denial pragma (Plan 0002 Phase 2, extended to scenes by Plan
// 0003 Phase 0). Steps + draws every displayed frame.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

use super::{Scene, SeededRng};
use crate::dsp::AnalysisFrame;
use crate::render::feedback::PingPongField;

/// Particle count. GPU-resident state is ~16 bytes each, so this is ~0.8 MB of
/// storage (negligible); the real ceiling is additive-blend fill rate at high
/// counts, an on-device iGPU concern routed to `docs/on-device-validation.md`
/// (ADR-0015 Risks). The headless capture tests draw this many instances at a
/// small size, which the software adapter handles briskly.
const PARTICLE_COUNT: u32 = 50_000;
/// Compute workgroup size (1D). 64 is a safe, portable default across DX12/Metal.
const WORKGROUP: u32 = 64;
/// Seeded initial scatter half-extent. Points converge onto the attractor within
/// a few iterations regardless, so a modest starting box is enough (NFR 6).
const INIT_SPREAD: f32 = 1.5;
const SEED: u64 = 0x4C4D_5641_5454_5231; // "LMVATTR1"

/// Fixed 16:9 accumulation grid the trails live on, decoupled from the surface
/// size (the reaction-diffusion resolution-independence discipline). Modest so
/// the extra decay+present fill the software capture pays stays cheap; the glow
/// is soft, so upscaling to 1080p reads fine.
const TRAIL_W: u32 = 640;
const TRAIL_H: u32 = 360;

/// Wall-clock duration of one attractor iteration (Plan 0014 injected `dt`). The
/// fixed-timestep accumulator runs one compute step per `FIXED_STEP` of injected
/// real `dt`, so the cloud evolves at the same rate on any refresh — at the
/// live/capture `dt` of 1/60 s this is exactly one step per frame. Continuous
/// (ODE) families added later integrate by this fixed sub-step, so the map is
/// frame-rate-independent without the shader reading a clock.
const FIXED_STEP: f32 = 1.0 / 60.0;
/// Max steps encoded in one frame — a long stall drops its backlog rather than
/// queueing unbounded compute work (accumulator spiral-of-death guard, as the
/// reaction-diffusion scene does). One step per frame is the norm at 60 fps.
const MAX_SUBSTEPS: u32 = 6;

/// De Jong attractor coefficients (Phase 1 hardcodes this classic set; Phase 3
/// exposes them as named params). The map is bounded in ~[-2, 2].
const DEJONG_A: f32 = 1.641;
const DEJONG_B: f32 = 1.902;
const DEJONG_C: f32 = 0.316;
const DEJONG_D: f32 = 1.525;

/// Slow display rotation (rad/s) driven by the scene clock, so the cloud visibly
/// turns even when the point set saturates its footprint — the animation
/// liveness the differential tests require, independent of audio.
const SPIN_RATE: f32 = 0.18;

/// Parameter defaults — a calm idle look when nothing is bound.
const DEFAULT_SIZE: f32 = 1.0;
const DEFAULT_HUE: f32 = 0.0;
/// Trail persistence: the fraction of the accumulation retained per 1/60 s frame.
/// ~0.94 gives glowing trails that fade over ~1 s; `fade = 0` clears each frame
/// (trail-free). Applied frame-rate-independently (raised to the `dt`-relative
/// power), so the trail length is the same wall-clock duration on any refresh.
const DEFAULT_FADE: f32 = 0.94;
/// Base point half-size in world units (before the `size` multiplier), matching
/// the swarm's small-glowing-point scale.
const POINT_BASE: f32 = 0.006;
/// `reseed` rises past this to re-scatter the cloud once (edge-triggered, so a
/// sustained beat flag doesn't re-scatter every frame).
const RESEED_THRESHOLD: f32 = 0.5;

/// Compute step: iterate every particle through the attractor map once. Writes
/// the storage buffer in place; the draw pass then reads it as a vertex buffer.
const STEP_SHADER: &str = r#"
struct Particle {
    pos: vec2<f32>,
    age: f32,
    seed: f32,
}
@group(0) @binding(0) var<storage, read_write> particles: array<Particle>;

struct Step {
    coeffs: vec4<f32>, // a, b, c, d
    dt: f32,           // fixed sub-step seconds (for continuous families)
    family: u32,       // which attractor map (Phase 1: 0 = De Jong)
    count: u32,        // active particle count
    pad: u32,
}
@group(0) @binding(1) var<uniform> step: Step;

@compute @workgroup_size(64)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= step.count) {
        return;
    }
    let a = step.coeffs.x;
    let b = step.coeffs.y;
    let c = step.coeffs.z;
    let d = step.coeffs.w;
    let p = particles[i].pos;
    // De Jong map: x' = sin(a*y) - cos(b*x), y' = sin(c*x) - cos(d*y).
    let nx = sin(a * p.y) - cos(b * p.x);
    let ny = sin(c * p.x) - cos(d * p.y);
    particles[i].pos = vec2<f32>(nx, ny);
    particles[i].age = particles[i].age + 1.0;
}
"#;

/// Draw pass: one additive glowing point-sprite per particle, into the trail
/// field. The particle storage buffer is bound as an instance vertex buffer; the
/// shader expands each into a screen-facing quad and tints it from the seeded
/// per-particle offset.
const DRAW_SHADER: &str = r#"
struct Draw {
    // x: aspect, y: point half-size (world), z: hue offset, w: display rotation
    v: vec4<f32>,
}
@group(0) @binding(0) var<uniform> draw: Draw;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) color: vec3<f32>,
}

// iq-style cosine palette (RGB phase-shifted), matching the swarm/fragment look.
fn palette(t: f32) -> vec3<f32> {
    let tau = 6.28318530718;
    return vec3<f32>(
        0.5 + 0.5 * cos(tau * (t + 0.10)),
        0.5 + 0.5 * cos(tau * (t + 0.42)),
        0.5 + 0.5 * cos(tau * (t + 0.62)),
    );
}

@vertex
fn vs_main(
    @builtin(vertex_index) vi: u32,
    @location(0) center: vec2<f32>,
    @location(1) age: f32,
    @location(2) seed: f32,
) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let corner = corners[vi] * 2.0 - vec2<f32>(1.0, 1.0);
    let aspect = draw.v.x;
    let psize = draw.v.y;
    let hue = draw.v.z;
    let rot = draw.v.w;

    let cs = cos(rot);
    let sn = sin(rot);
    let r = vec2<f32>(center.x * cs - center.y * sn, center.x * sn + center.y * cs);
    let world = r * 0.42 + corner * psize;

    var out: VsOut;
    out.pos = vec4<f32>(world.x / aspect, world.y, 0.0, 1.0);
    out.local = corner;
    out.color = palette(hue + seed * 0.15);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let d = length(in.local);
    let falloff = max(0.0, 1.0 - d);
    let g = falloff * falloff;
    return vec4<f32>(in.color * g, 1.0);
}
"#;

/// Decay pass: draw the previous accumulation back into the fresh target scaled
/// by the per-frame retention factor `k`, laying down the faded trail before the
/// new points are added on top.
const DECAY_SHADER: &str = r#"
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
    out.uv = p * 0.5 + vec2<f32>(0.5, 0.5);
    return out;
}

struct Decay { k: vec4<f32> } // x: per-frame retention factor
@group(0) @binding(0) var prev: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> decay: Decay;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(prev, samp, in.uv, 0.0).rgb * decay.k.x;
    return vec4<f32>(c, 1.0);
}
"#;

/// Present pass: composite the accumulation field to the surface (linear sample,
/// stretched to fill; aspect ignored as in the reaction-diffusion present).
const PRESENT_SHADER: &str = r#"
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
    out.uv = p * 0.5 + vec2<f32>(0.5, 0.5);
    return out;
}

@group(0) @binding(0) var field: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let c = textureSampleLevel(field, samp, in.uv, 0.0).rgb;
    return vec4<f32>(c, 1.0);
}
"#;

/// One particle, GPU storage-buffer layout (std430). 16 bytes: a 2D attractor
/// position, an age counter, and a per-particle seed jitter set once at init.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Particle {
    pos: [f32; 2],
    age: f32,
    seed: f32,
}

/// Compute step uniform (per frame): the attractor coefficients, the fixed
/// sub-step `dt`, the selected family, and the active particle count.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct StepUniform {
    coeffs: [f32; 4],
    dt: f32,
    family: u32,
    count: u32,
    pad: u32,
}

/// Draw uniform (per frame): x aspect, y point half-size, z hue offset, w
/// display rotation.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DrawUniform {
    v: [f32; 4],
}

/// Decay uniform (per frame): x is the per-frame trail retention factor.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct DecayUniform {
    k: [f32; 4],
}

/// The GPU-side state, built lazily on first render (see the module docs).
struct Resources {
    compute_pipeline: wgpu::ComputePipeline,
    draw_pipeline: wgpu::RenderPipeline,
    decay_pipeline: wgpu::RenderPipeline,
    present_pipeline: wgpu::RenderPipeline,
    /// Two-texture accumulation the trails ping-pong between (ADR-0012 reuse).
    field: PingPongField,
    particles: wgpu::Buffer,
    step_uniform: wgpu::Buffer,
    draw_uniform: wgpu::Buffer,
    decay_uniform: wgpu::Buffer,
    compute_bg: wgpu::BindGroup,
    draw_bg: wgpu::BindGroup,
    /// Decay/present bind groups reading texture A / texture B — selected by the
    /// field's read side each frame so nothing is rebuilt on the hot path.
    decay_bg_a: wgpu::BindGroup,
    decay_bg_b: wgpu::BindGroup,
    present_bg_a: wgpu::BindGroup,
    present_bg_b: wgpu::BindGroup,
}

impl Resources {
    fn build(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let step_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("attractor-step-shader"),
            source: wgpu::ShaderSource::Wgsl(STEP_SHADER.into()),
        });
        let draw_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("attractor-draw-shader"),
            source: wgpu::ShaderSource::Wgsl(DRAW_SHADER.into()),
        });
        let decay_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("attractor-decay-shader"),
            source: wgpu::ShaderSource::Wgsl(DECAY_SHADER.into()),
        });
        let present_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("attractor-present-shader"),
            source: wgpu::ShaderSource::Wgsl(PRESENT_SHADER.into()),
        });

        let field = PingPongField::new(device, TRAIL_W, TRAIL_H);

        // Particle storage buffer: written by the compute step (STORAGE), read by
        // the draw pass as an instance vertex buffer (VERTEX), seeded once from
        // the CPU (COPY_DST). One buffer, two roles — no CPU round-trip.
        let particles = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("attractor-particles"),
            size: (PARTICLE_COUNT as usize * std::mem::size_of::<Particle>()) as u64,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let step_uniform =
            uniform_buffer(device, "attractor-step-uniform", size_of::<StepUniform>());
        let draw_uniform =
            uniform_buffer(device, "attractor-draw-uniform", size_of::<DrawUniform>());
        let decay_uniform =
            uniform_buffer(device, "attractor-decay-uniform", size_of::<DecayUniform>());

        // --- compute: read_write storage + step uniform ---
        let compute_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("attractor-compute-layout"),
            entries: &[
                storage_entry(0),
                uniform_entry(1, wgpu::ShaderStages::COMPUTE),
            ],
        });
        let compute_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("attractor-compute-bg"),
            layout: &compute_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: particles.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: step_uniform.as_entire_binding(),
                },
            ],
        });
        let compute_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("attractor-compute-pipeline-layout"),
                bind_group_layouts: &[Some(&compute_layout)],
                immediate_size: 0,
            });
        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("attractor-compute-pipeline"),
            layout: Some(&compute_pipeline_layout),
            module: &step_shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });

        // --- draw: the particle buffer as an instance vertex buffer, additively
        // into the trail field (float target so the accumulation has headroom) ---
        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("attractor-draw-layout"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX)],
        });
        let draw_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("attractor-draw-bg"),
            layout: &draw_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: draw_uniform.as_entire_binding(),
            }],
        });
        let draw_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("attractor-draw-pipeline-layout"),
            bind_group_layouts: &[Some(&draw_layout)],
            immediate_size: 0,
        });
        let draw_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("attractor-draw-pipeline"),
            layout: Some(&draw_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &draw_shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Particle>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2, // pos
                        1 => Float32,   // age
                        2 => Float32,   // seed
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &draw_shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: PingPongField::FORMAT,
                    // Additive: overlapping points bloom brighter (the dense look).
                    blend: Some(wgpu::BlendState {
                        color: wgpu::BlendComponent {
                            src_factor: wgpu::BlendFactor::One,
                            dst_factor: wgpu::BlendFactor::One,
                            operation: wgpu::BlendOperation::Add,
                        },
                        alpha: wgpu::BlendComponent::OVER,
                    }),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        // --- decay + present: fullscreen samples of the accumulation field ---
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("attractor-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let decay_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("attractor-decay-layout"),
            entries: &[
                texture_entry(0),
                sampler_entry(1),
                uniform_entry(2, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        let decay_bg_a = blit_bind_group(
            device,
            &decay_layout,
            "attractor-decay-bg-a",
            field.view_a(),
            &sampler,
            Some(&decay_uniform),
        );
        let decay_bg_b = blit_bind_group(
            device,
            &decay_layout,
            "attractor-decay-bg-b",
            field.view_b(),
            &sampler,
            Some(&decay_uniform),
        );
        let decay_pipeline = fullscreen_pipeline(
            device,
            &decay_shader,
            &decay_layout,
            PingPongField::FORMAT,
            "attractor-decay",
        );

        let present_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("attractor-present-layout"),
            entries: &[texture_entry(0), sampler_entry(1)],
        });
        let present_bg_a = blit_bind_group(
            device,
            &present_layout,
            "attractor-present-bg-a",
            field.view_a(),
            &sampler,
            None,
        );
        let present_bg_b = blit_bind_group(
            device,
            &present_layout,
            "attractor-present-bg-b",
            field.view_b(),
            &sampler,
            None,
        );
        let present_pipeline = fullscreen_pipeline(
            device,
            &present_shader,
            &present_layout,
            surface_format,
            "attractor-present",
        );

        Self {
            compute_pipeline,
            draw_pipeline,
            decay_pipeline,
            present_pipeline,
            field,
            particles,
            step_uniform,
            draw_uniform,
            decay_uniform,
            compute_bg,
            draw_bg,
            decay_bg_a,
            decay_bg_b,
            present_bg_a,
            present_bg_b,
        }
    }

    /// Clear both accumulation textures to black — run once after a (re)build so
    /// the first decay pass reads a defined (empty) trail rather than garbage.
    fn clear_field(&self, encoder: &mut wgpu::CommandEncoder) {
        for view in [self.field.view_a(), self.field.view_b()] {
            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("attractor-clear-pass"),
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

fn uniform_buffer(device: &wgpu::Device, label: &str, size: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some(label),
        size: size as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

fn storage_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: false },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn uniform_entry(binding: u32, visibility: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
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

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

/// A texture(+sampler)[+uniform] bind group for the decay/present fullscreen
/// passes. `uniform` is `Some` for decay (the retention factor) and `None` for
/// present (no scaling).
fn blit_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    label: &str,
    input: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    uniform: Option<&wgpu::Buffer>,
) -> wgpu::BindGroup {
    let mut entries = vec![
        wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(input),
        },
        wgpu::BindGroupEntry {
            binding: 1,
            resource: wgpu::BindingResource::Sampler(sampler),
        },
    ];
    if let Some(buf) = uniform {
        entries.push(wgpu::BindGroupEntry {
            binding: 2,
            resource: buf.as_entire_binding(),
        });
    }
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &entries,
    })
}

/// A fullscreen-triangle pipeline (no vertex buffers) writing into `target`.
fn fullscreen_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    bind_layout: &wgpu::BindGroupLayout,
    target: wgpu::TextureFormat,
    label: &str,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[Some(bind_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
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
                format: target,
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

/// GPU compute-particle strange-attractor scene (ADR-0015). A storage buffer of
/// particles is stepped through the De Jong map by a compute shader each frame,
/// drawn as additive point-sprites into a fading trail field, and composited to
/// the surface. Every knob is an ADR-0002 layer-2 named parameter — the attractor
/// coefficients (`a`,`b`,`c`,`d`), the look scalars (`size`,`hue`,`fade`), and a
/// beat-driven `reseed` — so a preset binds them to the audio bands and beat.
pub struct AttractorScene {
    /// Cloned device handle (an `Arc` inside wgpu) used to build [`Resources`]
    /// lazily on first render — see the module docs for why.
    device: wgpu::Device,
    surface_format: wgpu::TextureFormat,
    res: Option<Resources>,
    /// The deterministic seeded scatter, uploaded on the first frame after a
    /// (re)build so a rebuilt scene restarts identically (capture determinism).
    seed_particles: Vec<Particle>,
    /// Re-upload the seed scatter next render. Set on first build and by a
    /// `reseed` rising edge (a beat re-scatters the cloud, blooming through the
    /// trails). The seed is fixed, so a re-scatter stays deterministic.
    needs_upload: bool,
    /// Clear the trail field to black next render. Set only on first build (not on
    /// reseed, so a beat's re-scatter blooms over the existing trails).
    needs_clear: bool,
    /// Fixed-timestep accumulator: unspent injected `dt`, drained one
    /// [`FIXED_STEP`] at a time into compute steps.
    accumulator: f32,
    /// Steps `advance` scheduled for the next `render` to encode.
    pending_steps: u32,
    /// Real elapsed seconds for this frame, injected via `advance`, used to make
    /// the trail decay frame-rate-independent.
    dt: f32,
    /// Shared scene clock (seconds), set by the renderer each frame.
    time: f32,
    /// Attractor coefficients (De Jong `a`,`b`,`c`,`d`) — named params, so a
    /// preset can steer the cloud's shape with the bands.
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    size: f32,
    hue: f32,
    fade: f32,
    /// This frame's `reseed` level (bound to a beat/onset expression); its rising
    /// edge re-scatters the cloud.
    reseed: f32,
    /// Previous frame's `reseed`, for rising-edge detection.
    prev_reseed: f32,
}

impl AttractorScene {
    /// Build the CPU-side seeded scatter. GPU resources are deferred to the first
    /// render (module docs).
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let seed_particles = Self::seed();
        Self {
            device: device.clone(),
            surface_format,
            res: None,
            seed_particles,
            needs_upload: true,
            needs_clear: true,
            accumulator: 0.0,
            pending_steps: 0,
            dt: FIXED_STEP,
            time: 0.0,
            a: DEJONG_A,
            b: DEJONG_B,
            c: DEJONG_C,
            d: DEJONG_D,
            size: DEFAULT_SIZE,
            hue: DEFAULT_HUE,
            fade: DEFAULT_FADE,
            reseed: 0.0,
            prev_reseed: 0.0,
        }
    }

    /// The deterministic initial particle set: a seeded scatter in a small box,
    /// each with a per-particle hue jitter. Points converge onto the attractor
    /// within a few iterations, so the starting positions only need to differ.
    fn seed() -> Vec<Particle> {
        let mut rng = SeededRng::new(SEED);
        (0..PARTICLE_COUNT)
            .map(|_| Particle {
                pos: [
                    rng.range(-INIT_SPREAD, INIT_SPREAD),
                    rng.range(-INIT_SPREAD, INIT_SPREAD),
                ],
                age: 0.0,
                seed: rng.next_f32(),
            })
            .collect()
    }
}

impl Scene for AttractorScene {
    fn name(&self) -> &'static str {
        "attractor"
    }

    fn advance(&mut self, dt: f32) {
        self.dt = dt;
        // Drain the accumulator one fixed step at a time, clamped so a long stall
        // can't queue unbounded compute work (the reaction-diffusion discipline).
        // The sub-`FIXED_STEP` remainder carries to the next frame.
        self.accumulator += dt;
        let mut steps = 0u32;
        while self.accumulator >= FIXED_STEP && steps < MAX_SUBSTEPS {
            self.accumulator -= FIXED_STEP;
            steps += 1;
        }
        self.accumulator = self.accumulator.min(FIXED_STEP);
        self.pending_steps = steps;
    }

    fn set_time(&mut self, time: f32) {
        self.time = time;
    }

    fn reset_params(&mut self) {
        // Defaults are the calm De Jong look, so an unbound preset (or a param a
        // preset leaves out) falls back here rather than leaking last frame's.
        self.a = DEJONG_A;
        self.b = DEJONG_B;
        self.c = DEJONG_C;
        self.d = DEJONG_D;
        self.size = DEFAULT_SIZE;
        self.hue = DEFAULT_HUE;
        self.fade = DEFAULT_FADE;
        self.reseed = 0.0;
    }

    fn set_param(&mut self, name: &str, value: f32) {
        match name {
            "a" => self.a = value,
            "b" => self.b = value,
            "c" => self.c = value,
            "d" => self.d = value,
            "size" => self.size = value,
            "hue" => self.hue = value,
            "fade" => self.fade = value,
            "reseed" => self.reseed = value,
            _ => {}
        }
    }

    fn update(&mut self, _frame: &AnalysisFrame) {
        // Rising-edge detect on `reseed` (a beat/onset expression): re-scatter the
        // cloud once. Edge-triggered so a sustained flag doesn't re-scatter every
        // frame; deterministic because the seed is fixed. The trail field is kept
        // (only particles re-upload), so the re-scatter blooms through the trails.
        if self.reseed >= RESEED_THRESHOLD && self.prev_reseed < RESEED_THRESHOLD {
            self.needs_upload = true;
        }
        self.prev_reseed = self.reseed;
    }

    fn render(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        _aspect: f32,
    ) {
        if self.res.is_none() {
            self.res = Some(Resources::build(&self.device, self.surface_format));
        }
        let Self {
            res,
            seed_particles,
            needs_upload,
            needs_clear,
            pending_steps,
            dt,
            time,
            a,
            b,
            c,
            d,
            size,
            hue,
            fade,
            ..
        } = self;
        let Some(res) = res.as_mut() else {
            return;
        };

        // Clear the trail field once after a (re)build so the first decay reads
        // black rather than garbage.
        if *needs_clear {
            res.clear_field(encoder);
            *needs_clear = false;
        }
        // (Re)upload the seeded scatter — on first build, and each time a `reseed`
        // rising edge re-scatters the cloud (the trail field is kept).
        if *needs_upload {
            queue.write_buffer(&res.particles, 0, bytemuck::cast_slice(seed_particles));
            *needs_upload = false;
        }

        queue.write_buffer(
            &res.step_uniform,
            0,
            bytemuck::bytes_of(&StepUniform {
                coeffs: [*a, *b, *c, *d],
                dt: FIXED_STEP,
                family: 0,
                count: PARTICLE_COUNT,
                pad: 0,
            }),
        );
        queue.write_buffer(
            &res.draw_uniform,
            0,
            bytemuck::bytes_of(&DrawUniform {
                v: [
                    TRAIL_W as f32 / TRAIL_H as f32,
                    POINT_BASE * *size,
                    *hue,
                    *time * SPIN_RATE,
                ],
            }),
        );
        // Frame-rate-independent trail decay: retain `fade` per 1/60 s, raised to
        // the `dt`-relative power so the trail length is the same wall-clock
        // duration on any refresh. `fade = 0` -> factor 0 -> trail-free.
        let decay = fade.clamp(0.0, 1.0).powf((*dt * 60.0).max(0.0));
        queue.write_buffer(
            &res.decay_uniform,
            0,
            bytemuck::bytes_of(&DecayUniform {
                k: [decay, 0.0, 0.0, 0.0],
            }),
        );

        // Step the particles: one compute dispatch per scheduled sub-step. wgpu
        // inserts the storage->vertex barrier before the draw pass below.
        let groups = PARTICLE_COUNT.div_ceil(WORKGROUP);
        for _ in 0..*pending_steps {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("attractor-step-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&res.compute_pipeline);
            pass.set_bind_group(0, &res.compute_bg, &[]);
            pass.dispatch_workgroups(groups, 1, 1);
        }

        // Trail pass: draw the faded previous accumulation into the fresh target,
        // then add this frame's points on top. One pass, so the decay lays the
        // bed and the additive points bloom over it. The decay reads the current
        // read side; the present below reads the freshly-written side after swap.
        let decay_bg = if res.field.reading_a() {
            &res.decay_bg_a
        } else {
            &res.decay_bg_b
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("attractor-trail-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: res.field.write_view(),
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
            pass.set_pipeline(&res.decay_pipeline);
            pass.set_bind_group(0, decay_bg, &[]);
            pass.draw(0..3, 0..1);

            pass.set_pipeline(&res.draw_pipeline);
            pass.set_bind_group(0, &res.draw_bg, &[]);
            pass.set_vertex_buffer(0, res.particles.slice(..));
            pass.draw(0..6, 0..PARTICLE_COUNT);
        }
        res.field.swap();

        // Present the freshly-written accumulation to the surface.
        let present_bg = if res.field.reading_a() {
            &res.present_bg_a
        } else {
            &res.present_bg_b
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("attractor-present-pass"),
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
        pass.set_pipeline(&res.present_pipeline);
        pass.set_bind_group(0, present_bg, &[]);
        pass.draw(0..3, 0..1);
    }
}
