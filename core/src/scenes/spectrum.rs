//! Spectrum-bars scene: 64 log-frequency bars, instanced quads, no vertex
//! buffers. Bar heights rise instantly and decay smoothly; onsets flash the
//! background.

use super::Scene;
use crate::dsp::{AnalysisFrame, SPECTRUM_BINS};

/// Per-frame fall of a bar toward the live value (frame-rate coupled; fine
/// for the fixed-quality MVP).
const DECAY: f32 = 0.92;
/// Perceptual lift: amplitudes are compressed with sqrt so quiet content
/// still shows structure.
const ONSET_FLASH_GAIN: f32 = 3.0;

const SHADER: &str = r#"
struct Params {
    // 64 bar heights packed into vec4s (uniform array stride rules).
    heights: array<vec4<f32>, 16>,
    // x: onset flash 0..1, yzw: padding
    misc: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) frac_up: f32,
    @location(1) frac_across: f32,
    @location(2) height: f32,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[vi];
    let n = 64.0;
    let gap = 0.18;
    let h = clamp(params.heights[ii / 4u][ii % 4u], 0.004, 1.0);

    let slot = 2.0 / n;
    let x0 = -1.0 + (f32(ii) + gap * 0.5) * slot;
    let w = slot * (1.0 - gap);
    let y0 = -0.92;
    let y = y0 + c.y * h * 1.84;

    var out: VsOut;
    out.pos = vec4<f32>(x0 + c.x * w, y, 0.0, 1.0);
    out.frac_up = c.y;
    out.frac_across = f32(ii) / n;
    out.height = h;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Cool-to-hot sweep across the spectrum, brighter toward the bar tip.
    let low = vec3<f32>(0.10, 0.55, 0.95);
    let high = vec3<f32>(0.95, 0.25, 0.55);
    let base = mix(low, high, in.frac_across);
    let lift = 0.30 + 0.70 * in.frac_up;
    let flash = params.misc.x;
    let color = base * lift * (1.0 + 0.6 * flash) + vec3<f32>(flash * 0.08);
    return vec4<f32>(color, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    heights: [[f32; 4]; 16],
    misc: [f32; 4],
}

/// Spectrum-bars scene: 64 instanced log-frequency bars with onset flash.
pub struct SpectrumScene {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    smoothed: [f32; SPECTRUM_BINS],
    flash: f32,
}

impl SpectrumScene {
    /// Build the scene's pipeline and uniform buffers on `device`.
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spectrum-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("spectrum-params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("spectrum-bind-layout"),
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
            label: Some("spectrum-bind-group"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spectrum-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("spectrum-pipeline"),
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
            smoothed: [0.0; SPECTRUM_BINS],
            flash: 0.0,
        }
    }
}

impl Scene for SpectrumScene {
    fn name(&self) -> &'static str {
        "spectrum"
    }

    fn update(&mut self, frame: &AnalysisFrame) {
        for (s, &v) in self.smoothed.iter_mut().zip(frame.spectrum.iter()) {
            let v = v.clamp(0.0, 1.0).sqrt();
            *s = v.max(*s * DECAY);
        }
        self.flash = (frame.onset * ONSET_FLASH_GAIN)
            .clamp(0.0, 1.0)
            .max(self.flash * DECAY);
    }

    fn render(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        _aspect: f32,
    ) {
        let mut params = Params {
            heights: [[0.0; 4]; 16],
            misc: [self.flash, 0.0, 0.0, 0.0],
        };
        for (i, &s) in self.smoothed.iter().enumerate() {
            params.heights[i / 4][i % 4] = s;
        }
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&params));

        let bg = 0.02 + 0.05 * self.flash as f64;
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("spectrum-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: bg,
                        g: bg,
                        b: bg * 1.6,
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
        pass.draw(0..6, 0..SPECTRUM_BINS as u32);
    }
}
