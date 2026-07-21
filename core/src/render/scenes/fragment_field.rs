//! Fragment-field scene: a fullscreen Shadertoy-style domain-warped field,
//! colored by a cosine palette and driven by the audio analysis. This is the
//! first "generative-art"-tier built-in (Plan 0003 Phase 1) and the system the
//! preset layer will later parameterize (ADR-0002 layers 1-2).
//!
//! Phase 1 drives it from the *existing* [`AnalysisFrame`] (spectrum + onset +
//! beat) via band proxies computed from the spectrum; Phase 2 swaps those
//! proxies for the analyzer's real `bass`/`mid`/`treb`. The uniform layout is
//! kept stable across that swap.

// Hot-path panic-denial pragma (Plan 0002 Phase 2, extended to scenes by Plan
// 0003 Phase 0). Runs every displayed frame.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

use super::{SCENE_DT, Scene};
use crate::dsp::AnalysisFrame;

/// Smoothing decay for the band/energy envelopes (per frame; frame-rate
/// coupled, fine for the fixed-quality MVP like the sibling scenes).
const DECAY: f32 = 0.90;
/// Onset-flash gain before clamping to 0..1.
const ONSET_FLASH_GAIN: f32 = 3.0;
/// Per-frame decay of the discrete beat "kick".
const KICK_DECAY: f32 = 0.88;

const SHADER: &str = r#"
struct Params {
    // x: time (s), y: aspect, z: onset flash 0..1, w: beat kick 0..1 (decaying)
    misc: vec4<f32>,
    // x: bass, y: mid, z: treb, w: overall energy (all smoothed, ~0..1)
    bands: vec4<f32>,
}

@group(0) @binding(0) var<uniform> params: Params;

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

// iq-style cosine palette: smooth, loops in hue.
fn palette(t: f32) -> vec3<f32> {
    let a = vec3<f32>(0.5, 0.5, 0.5);
    let b = vec3<f32>(0.5, 0.5, 0.5);
    let c = vec3<f32>(1.0, 1.0, 1.0);
    let d = vec3<f32>(0.10, 0.42, 0.62);
    return a + b * cos(6.28318 * (c * t + d));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let t = params.misc.x;
    let aspect = params.misc.y;
    let flash = params.misc.z;
    let kick = params.misc.w;
    let bass = params.bands.x;
    let mid = params.bands.y;
    let treb = params.bands.z;
    let energy = params.bands.w;

    var uv = in.ndc;
    uv.x = uv.x * aspect;

    // Iterated sine-fold domain warp; bass swells the fold amount, the beat
    // kick pushes an extra shove so drops visibly bloom.
    var p = uv * (1.4 + bass * 1.2);
    let warp = 0.30 + bass * 1.5 + kick * 0.6;
    for (var i = 0; i < 5; i = i + 1) {
        let fi = f32(i);
        p = p + warp * vec2<f32>(
            sin(p.y * 1.5 + t * 0.7 + fi),
            cos(p.x * 1.5 - t * 0.6 + fi)
        ) / (fi + 1.0);
    }

    let field = 0.5 + 0.5 * sin(p.x + p.y + t * 0.5);
    let hue = field * 0.6 + t * 0.04 + treb * 0.35 + mid * 0.1;
    var col = palette(hue);

    // Radial falloff, lifted by overall energy and the onset flash.
    let r = length(uv);
    let glow = (0.35 + energy * 1.5 + flash * 0.8) * (1.0 - 0.25 * r);
    col = col * glow;
    col = col + vec3<f32>(flash * 0.12);
    col = col * (1.0 + kick * 0.4);

    return vec4<f32>(col, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    misc: [f32; 4],
    bands: [f32; 4],
}

/// Fullscreen domain-warped fragment field, audio-reactive via band proxies
/// and the onset/beat signals.
pub struct FragmentFieldScene {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// Scene clock (seconds), advanced by the fixed timestep — never the wall
    /// clock (determinism, NFR 6).
    time: f32,
    bass: f32,
    mid: f32,
    treb: f32,
    energy: f32,
    flash: f32,
    kick: f32,
}

impl FragmentFieldScene {
    /// Build the scene's pipeline and uniform buffer on `device`.
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fragment-field-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fragment-field-params"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fragment-field-bind-layout"),
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
            label: Some("fragment-field-bind-group"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fragment-field-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("fragment-field-pipeline"),
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
            time: 0.0,
            bass: 0.0,
            mid: 0.0,
            treb: 0.0,
            energy: 0.0,
            flash: 0.0,
            kick: 0.0,
        }
    }
}

/// Mean of a spectrum sub-range, perceptually compressed with sqrt so quiet
/// content still registers. Uses iterators (no indexing) so it stays inside
/// the panic pragma without an allow.
fn band_mean(frame: &AnalysisFrame, skip: usize, take: usize) -> f32 {
    let sum: f32 = frame.spectrum.iter().skip(skip).take(take).sum();
    (sum / take.max(1) as f32).clamp(0.0, 1.0).sqrt()
}

impl Scene for FragmentFieldScene {
    fn name(&self) -> &'static str {
        "fragment field"
    }

    fn update(&mut self, frame: &AnalysisFrame) {
        self.time += SCENE_DT;

        // Band proxies from the existing spectrum (64 log bins): low / mid /
        // high thirds. Phase 2 replaces these with the analyzer's real bands.
        let bass = band_mean(frame, 0, 8);
        let mid = band_mean(frame, 8, 24);
        let treb = band_mean(frame, 32, 32);
        let energy = band_mean(frame, 0, 64);

        // Attack instantly, decay smoothly — the sibling scenes' envelope feel.
        self.bass = bass.max(self.bass * DECAY);
        self.mid = mid.max(self.mid * DECAY);
        self.treb = treb.max(self.treb * DECAY);
        self.energy = energy.max(self.energy * DECAY);

        self.flash = (frame.onset * ONSET_FLASH_GAIN)
            .clamp(0.0, 1.0)
            .max(self.flash * DECAY);
        self.kick = if frame.beat {
            1.0
        } else {
            self.kick * KICK_DECAY
        };
    }

    fn render(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        aspect: f32,
    ) {
        let params = Params {
            misc: [self.time, aspect.max(0.1), self.flash, self.kick],
            bands: [self.bass, self.mid, self.treb, self.energy],
        };
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&params));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("fragment-field-pass"),
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
