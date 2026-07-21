//! Starfield scene: seeded particles streaming outward. Overall energy sets
//! the cruise speed, beats kick a speed burst, treble brightens the stars.
//! Randomness comes only from the explicitly seeded RNG (NFR 6).

use super::{SCENE_DT, Scene, SeededRng, energy_level};
use crate::dsp::AnalysisFrame;

const PARTICLES: usize = 320;
/// World bounds: y spans -1.1..1.1, x wide enough for ultrawide windows.
const BOUND_X: f32 = 2.0;
const BOUND_Y: f32 = 1.1;
const SEED: u64 = 0x4C4D_565F_5354_4152; // "LMV_STAR"

const SHADER: &str = r#"
struct Misc {
    // x: aspect, yzw: unused
    v: vec4<f32>,
}

@group(0) @binding(0) var<uniform> misc: Misc;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) local: vec2<f32>,
    @location(1) bright: f32,
}

@vertex
fn vs_main(
    @builtin(vertex_index) vi: u32,
    @location(0) center: vec2<f32>,
    @location(1) size: f32,
    @location(2) bright: f32,
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
    out.bright = bright;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let d = length(in.local);
    let falloff = max(0.0, 1.0 - d);
    let g = falloff * falloff * in.bright;
    return vec4<f32>(vec3<f32>(0.70, 0.85, 1.00) * g, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Instance {
    center: [f32; 2],
    size: f32,
    bright: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Misc {
    v: [f32; 4],
}

struct Particle {
    pos: [f32; 2],
    dir: [f32; 2],
    speed: f32,
    size: f32,
    base_bright: f32,
}

pub struct StarfieldScene {
    pipeline: wgpu::RenderPipeline,
    instances: wgpu::Buffer,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    particles: Vec<Particle>,
    instance_data: Vec<Instance>,
    rng: SeededRng,
    burst: f32,
    energy_smoothed: f32,
}

impl StarfieldScene {
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("starfield-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("starfield-instances"),
            size: (PARTICLES * std::mem::size_of::<Instance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("starfield-misc"),
            size: std::mem::size_of::<Misc>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("starfield-bind-layout"),
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
            label: Some("starfield-bind-group"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("starfield-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("starfield-pipeline"),
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
                        2 => Float32,
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    // Additive: overlapping stars glow brighter.
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
        let particles = (0..PARTICLES)
            .map(|_| Self::spawn(&mut rng, true))
            .collect();

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
                    bright: 0.0
                };
                PARTICLES
            ],
            rng,
            burst: 0.0,
            energy_smoothed: 0.0,
        }
    }

    /// New particle heading outward. Initial fill scatters over the whole
    /// field; respawns start near the center.
    fn spawn(rng: &mut SeededRng, scatter: bool) -> Particle {
        let angle = rng.range(0.0, std::f32::consts::TAU);
        let dir = [angle.cos(), angle.sin()];
        let radius = if scatter {
            rng.range(0.05, 1.0)
        } else {
            rng.range(0.02, 0.25)
        };
        Particle {
            pos: [dir[0] * radius, dir[1] * radius],
            dir,
            speed: rng.range(0.5, 1.5),
            size: rng.range(0.004, 0.016),
            base_bright: rng.range(0.4, 1.0),
        }
    }
}

impl Scene for StarfieldScene {
    fn name(&self) -> &'static str {
        "starfield"
    }

    fn update(&mut self, frame: &AnalysisFrame) {
        if frame.beat {
            self.burst = 1.0;
        } else {
            self.burst *= 0.90;
        }
        let energy = energy_level(frame).sqrt();
        self.energy_smoothed = energy.max(self.energy_smoothed * 0.95);

        let speed_scale = 0.12 + self.energy_smoothed * 1.2 + self.burst * 1.5;
        let bright_scale = 0.35 + self.energy_smoothed * 1.2 + self.burst * 0.8;

        for (particle, inst) in self.particles.iter_mut().zip(self.instance_data.iter_mut()) {
            particle.pos[0] += particle.dir[0] * particle.speed * speed_scale * SCENE_DT;
            particle.pos[1] += particle.dir[1] * particle.speed * speed_scale * SCENE_DT;
            if particle.pos[0].abs() > BOUND_X || particle.pos[1].abs() > BOUND_Y {
                *particle = Self::spawn(&mut self.rng, false);
            }
            // Stars farther out read brighter — cheap depth cue.
            let dist = (particle.pos[0] * particle.pos[0] + particle.pos[1] * particle.pos[1])
                .sqrt()
                .min(1.0);
            *inst = Instance {
                center: particle.pos,
                size: particle.size * (0.6 + dist),
                bright: (particle.base_bright * bright_scale * (0.4 + 0.6 * dist)).min(1.5),
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
            label: Some("starfield-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.004,
                        g: 0.004,
                        b: 0.012,
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
