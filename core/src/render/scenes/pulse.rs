//! Beat-pulse scene: every detected beat spawns an expanding ring; bass
//! feeds a center glow. Overtly beat-driven — the reactivity showcase.

// Hot-path panic-denial pragma (Plan 0002 Phase 2, extended to scenes by Plan
// 0003 Phase 0). Runs every displayed frame.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

use super::{SCENE_DT, Scene, bass_level};
use crate::dsp::AnalysisFrame;

const MAX_RINGS: usize = 8;
/// Ring expansion speed in NDC units per second.
const RING_SPEED: f32 = 1.6;

const SHADER: &str = r#"
struct Params {
    // x: age (s), y: intensity, zw: unused
    rings: array<vec4<f32>, 8>,
    // x: bass, y: aspect, zw: unused
    misc: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var pts = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(pts[vi], 0.0, 1.0);
    out.ndc = pts[vi];
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var p = in.ndc;
    p.x = p.x * params.misc.y;
    let r = length(p);

    var glow = 0.0;
    for (var i = 0u; i < 8u; i = i + 1u) {
        let ring = params.rings[i];
        if (ring.y <= 0.0) { continue; }
        let radius = ring.x * 1.6;
        let d = abs(r - radius);
        glow = glow + ring.y * exp(-d * 30.0) * exp(-ring.x * 1.8);
    }

    let bass = params.misc.x;
    let center = exp(-r * 4.0) * bass * 1.2;

    let ring_color = vec3<f32>(0.95, 0.35, 0.55);
    let core_color = vec3<f32>(0.20, 0.55, 1.00);
    let bgc = vec3<f32>(0.012, 0.012, 0.028);
    let color = ring_color * glow + core_color * center + bgc;
    return vec4<f32>(color, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    rings: [[f32; 4]; MAX_RINGS],
    misc: [f32; 4],
}

/// Beat-driven scene: expanding rings on each beat over a bass-fed glow.
pub struct PulseScene {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// (age seconds, intensity) per slot; intensity 0 = free.
    rings: [[f32; 4]; MAX_RINGS],
    bass_smoothed: f32,
}

impl PulseScene {
    /// Build the scene's pipeline and uniform buffers on `device`.
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pulse-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("pulse-params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("pulse-bind-layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("pulse-bind-group"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pulse-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pulse-pipeline"),
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
            pipeline,
            uniforms,
            bind_group,
            rings: [[0.0; 4]; MAX_RINGS],
            bass_smoothed: 0.0,
        }
    }
}

impl Scene for PulseScene {
    fn name(&self) -> &'static str {
        "pulse"
    }

    #[allow(
        clippy::indexing_slicing,
        reason = "ring[0]/ring[1] index fixed [f32; 4] slots (0,1 constant); slot is a position()/max_by() result, always < MAX_RINGS"
    )]
    fn update(&mut self, frame: &AnalysisFrame) {
        for ring in self.rings.iter_mut() {
            if ring[1] > 0.0 {
                ring[0] += SCENE_DT;
                // Retire once fully expanded off-screen.
                if ring[0] * RING_SPEED > 2.5 {
                    ring[1] = 0.0;
                }
            }
        }
        if frame.beat {
            // Take the free slot, else recycle the oldest ring.
            let slot = self
                .rings
                .iter()
                .position(|r| r[1] <= 0.0)
                .unwrap_or_else(|| {
                    self.rings
                        .iter()
                        .enumerate()
                        .max_by(|a, b| a.1[0].total_cmp(&b.1[0]))
                        .map(|(i, _)| i)
                        .unwrap_or(0)
                });
            self.rings[slot] = [0.0, (0.5 + frame.onset * 3.0).clamp(0.5, 1.0), 0.0, 0.0];
        }
        let bass = bass_level(frame).sqrt();
        self.bass_smoothed = bass.max(self.bass_smoothed * 0.90);
    }

    fn render(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        aspect: f32,
    ) {
        let params = Params {
            rings: self.rings,
            misc: [self.bass_smoothed, aspect, 0.0, 0.0],
        };
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&params));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("pulse-pass"),
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
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }
}
