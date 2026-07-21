//! Particle-swarm scene: ~10k CPU-simulated particles drifting through an
//! audio-driven flow field, drawn as instanced additive sprites (the
//! starfield's rendering approach, scaled up and generalized).
//!
//! Bass swells the flow force, the beat fires an outward burst, the bar phase
//! and mid band evolve the field, and treble shifts the palette. All per-
//! particle math is CPU-side — no compute shader (Plan 0003 Phase 3). Motion
//! is deterministic; the only randomness is the explicitly seeded initial
//! scatter (NFR 6).

// Hot-path panic-denial pragma (Plan 0002 Phase 2, extended to scenes by Plan
// 0003 Phase 0). Runs every displayed frame.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

use super::{SCENE_DT, Scene, SeededRng};
use crate::dsp::AnalysisFrame;

/// Particle count. 10k is the target look (Plan 0003); it holds the primary
/// dev box comfortably and is the number to validate against the 60 fps @
/// 1080p floor on the iGPU test PC (NFR 1/9), reducing here if it misses.
const PARTICLES: usize = 10_000;
/// Toroidal world half-extents; x is wider so the field fills 16:9 after the
/// shader's aspect divide (matches the starfield's convention).
const BOUND_X: f32 = 1.8;
const BOUND_Y: f32 = 1.0;
const SEED: u64 = 0x4C4D_565F_5357_524D; // "LMV_SWRM"

/// Velocity retained per frame (the rest is re-steered by the flow field).
const DAMPING: f32 = 0.86;
/// Spatial frequency of the flow field.
const FIELD_FREQ: f32 = 2.3;
/// Baseline steering force and the extra the bass band adds.
const BASE_FORCE: f32 = 1.4;
const BASS_FORCE: f32 = 3.0;
/// Outward radial impulse a beat injects (scaled by the decaying burst).
const BURST_FORCE: f32 = 4.0;
/// Band gain: raw band means are small (energy spread over many bins); lift
/// them into a usable 0..1 drive range. A display knob, like the fragment
/// scene's — Phase 5 presets will expose it.
const BAND_GAIN: f32 = 8.0;

const SHADER: &str = r#"
struct Misc {
    // x: aspect, yzw: unused
    v: vec4<f32>,
}

@group(0) @binding(0) var<uniform> misc: Misc;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) color: vec3<f32>,
}

@vertex
fn vs_main(
    @builtin(vertex_index) vi: u32,
    @location(0) center: vec2<f32>,
    @location(1) size: f32,
    @location(2) color: vec3<f32>,
) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[vi] * 2.0 - vec2<f32>(1.0, 1.0);
    let world = center + c * size;
    var out: VsOut;
    out.pos = vec4<f32>(world.x / misc.v.x, world.y, 0.0, 1.0);
    out.local = c;
    out.color = color;
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

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    center: [f32; 2],
    size: f32,
    color: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Misc {
    v: [f32; 4],
}

struct Particle {
    pos: [f32; 2],
    vel: [f32; 2],
    /// Per-particle palette offset and brightness, from the seeded scatter.
    hue: f32,
    bright: f32,
    size: f32,
}

/// ~10k-particle CPU flow-field swarm with instanced additive rendering.
pub struct SwarmScene {
    pipeline: wgpu::RenderPipeline,
    instances: wgpu::Buffer,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    particles: Vec<Particle>,
    instance_data: Vec<Instance>,
    /// Scene clock (seconds), fixed timestep — never the wall clock (NFR 6).
    time: f32,
    bass: f32,
    mid: f32,
    treb: f32,
    burst: f32,
    bar: f32,
}

impl SwarmScene {
    /// Build the pipeline, buffers, and seeded particle set on `device`.
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("swarm-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("swarm-instances"),
            size: (PARTICLES * std::mem::size_of::<Instance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("swarm-misc"),
            size: std::mem::size_of::<Misc>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("swarm-bind-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("swarm-bind-group"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("swarm-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("swarm-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Instance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32,
                        2 => Float32x3,
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    // Additive: overlapping particles bloom brighter.
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

        let mut rng = SeededRng::new(SEED);
        let particles = (0..PARTICLES).map(|_| Self::spawn(&mut rng)).collect();

        Self {
            pipeline,
            instances,
            uniforms,
            bind_group,
            particles,
            instance_data: vec![
                Instance {
                    center: [0.0, 0.0],
                    size: 0.0,
                    color: [0.0, 0.0, 0.0],
                };
                PARTICLES
            ],
            time: 0.0,
            bass: 0.0,
            mid: 0.0,
            treb: 0.0,
            burst: 0.0,
            bar: 0.0,
        }
    }

    /// A particle scattered across the field with a random heading and tint.
    #[allow(
        clippy::indexing_slicing,
        reason = "pos/vel index a fixed [f32; 2] at constant 0/1, always in-bounds"
    )]
    fn spawn(rng: &mut SeededRng) -> Particle {
        let angle = rng.range(0.0, std::f32::consts::TAU);
        Particle {
            pos: [rng.range(-BOUND_X, BOUND_X), rng.range(-BOUND_Y, BOUND_Y)],
            vel: [angle.cos() * 0.2, angle.sin() * 0.2],
            hue: rng.next_f32(),
            bright: rng.range(0.5, 1.0),
            size: rng.range(0.004, 0.011),
        }
    }
}

/// Perceptual lift of a raw band mean into a 0..1 drive range.
fn shape(level: f32) -> f32 {
    (level * BAND_GAIN).clamp(0.0, 1.0).sqrt()
}

/// iq-style cosine palette (RGB phase-shifted), matching the fragment field's.
fn palette(t: f32) -> [f32; 3] {
    let tau = std::f32::consts::TAU;
    [
        0.5 + 0.5 * (tau * (t + 0.10)).cos(),
        0.5 + 0.5 * (tau * (t + 0.42)).cos(),
        0.5 + 0.5 * (tau * (t + 0.62)).cos(),
    ]
}

impl Scene for SwarmScene {
    fn name(&self) -> &'static str {
        "swarm"
    }

    #[allow(
        clippy::indexing_slicing,
        reason = "pos/vel index fixed [f32; 2] and base indexes a fixed [f32; 3], all at constant offsets, always in-bounds"
    )]
    fn update(&mut self, frame: &AnalysisFrame) {
        self.time += SCENE_DT;
        self.bass = shape(frame.bass);
        self.mid = shape(frame.mid);
        self.treb = shape(frame.treb);
        self.burst = if frame.beat { 1.0 } else { self.burst * 0.88 };
        self.bar = frame.bar;

        // Field evolves faster with mids and the bar phase; bass steers harder.
        let field_t = self.time * (0.3 + self.mid * 0.6 + self.bar * 0.4);
        let force = BASE_FORCE + self.bass * BASS_FORCE;
        let burst_kick = self.burst * BURST_FORCE;
        let hue_shift = self.time * 0.02 + self.treb * 0.4;

        for (p, inst) in self.particles.iter_mut().zip(self.instance_data.iter_mut()) {
            // Scalar potential -> flow direction (cheap curl-ish field).
            let a = (p.pos[0] * FIELD_FREQ + field_t).sin()
                + (p.pos[1] * FIELD_FREQ - field_t * 0.8).cos();
            let dir = [a.cos(), a.sin()];

            p.vel[0] = p.vel[0] * DAMPING + dir[0] * force * SCENE_DT;
            p.vel[1] = p.vel[1] * DAMPING + dir[1] * force * SCENE_DT;

            // Beat burst pushes particles radially outward from center.
            if burst_kick > 0.0 {
                let r = (p.pos[0] * p.pos[0] + p.pos[1] * p.pos[1]).sqrt().max(1e-3);
                p.vel[0] += p.pos[0] / r * burst_kick * SCENE_DT;
                p.vel[1] += p.pos[1] / r * burst_kick * SCENE_DT;
            }

            p.pos[0] += p.vel[0] * SCENE_DT;
            p.pos[1] += p.vel[1] * SCENE_DT;

            // Toroidal wrap keeps the field populated (no respawns/hitches).
            if p.pos[0] > BOUND_X {
                p.pos[0] -= 2.0 * BOUND_X;
            } else if p.pos[0] < -BOUND_X {
                p.pos[0] += 2.0 * BOUND_X;
            }
            if p.pos[1] > BOUND_Y {
                p.pos[1] -= 2.0 * BOUND_Y;
            } else if p.pos[1] < -BOUND_Y {
                p.pos[1] += 2.0 * BOUND_Y;
            }

            let speed = (p.vel[0] * p.vel[0] + p.vel[1] * p.vel[1]).sqrt();
            let base = palette(p.hue + hue_shift);
            let bright = ((0.25 + speed * 0.7 + self.burst * 0.6) * p.bright).min(1.6);
            // Bass lifts everyone; treble tints toward the palette's high end.
            let lift = bright * (0.6 + self.bass * 0.8);

            *inst = Instance {
                center: p.pos,
                size: p.size * (1.0 + self.burst * 0.5),
                color: [base[0] * lift, base[1] * lift, base[2] * lift],
            };
        }
    }

    fn render(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        aspect: f32,
    ) {
        queue.write_buffer(
            &self.instances,
            0,
            bytemuck::cast_slice(&self.instance_data),
        );
        queue.write_buffer(
            &self.uniforms,
            0,
            bytemuck::bytes_of(&Misc {
                v: [aspect.max(0.1), 0.0, 0.0, 0.0],
            }),
        );

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("swarm-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.008,
                        g: 0.006,
                        b: 0.016,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.draw(0..6, 0..PARTICLES as u32);
    }
}
