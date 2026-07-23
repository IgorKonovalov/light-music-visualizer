//! Fragment-field scene: a fullscreen Shadertoy-style domain-warped field,
//! colored by a cosine palette. The first "generative-art"-tier built-in and
//! one of the two preset-driven systems (ADR-0002 layers 1-2).
//!
//! Its look is a set of named parameters — `warp`, `hue`, `zoom`, `glow`,
//! `flash` — that a preset binds to expressions over the audio analysis (Plan
//! 0003 Phase 5). With no preset the parameter defaults render a gentle idle
//! field. The scene reads no audio directly; all reactivity flows through the
//! parameter values.

// Hot-path panic-denial pragma (Plan 0002 Phase 2, extended to scenes by Plan
// 0003 Phase 0). Runs every displayed frame.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

use super::Scene;
use crate::dsp::AnalysisFrame;

/// Parameter defaults — a calm idle field when nothing is bound.
const DEFAULT_WARP: f32 = 0.4;
const DEFAULT_HUE: f32 = 0.0;
const DEFAULT_ZOOM: f32 = 1.0;
const DEFAULT_GLOW: f32 = 0.7;
const DEFAULT_FLASH: f32 = 0.0;
// Shared view transform (ADR-0018): `pan_*` offset the sampled field window. The
// field's existing `zoom` already scales the sample coordinates (its view-zoom in
// field space), so Phase 2 completes the ViewTransform here by adding pan.
const DEFAULT_PAN: f32 = 0.0;

const SHADER: &str = r#"
struct Params {
    // x: time (s), y: aspect, z: warp, w: hue
    a: vec4<f32>,
    // x: zoom, y: glow, z: flash, w: unused
    b: vec4<f32>,
    // xy: pan (field-space offset, ADR-0018), zw: unused
    c: vec4<f32>,
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
    let t = params.a.x;
    let aspect = params.a.y;
    let warp = params.a.z;
    let hue = params.a.w;
    let zoom = params.b.x;
    let glow = params.b.y;
    let flash = params.b.z;
    let pan = params.c.xy;

    var uv = in.ndc;
    uv.x = uv.x * aspect;

    // Iterated sine-fold domain warp, scaled by zoom and folded by warp; `pan`
    // slides the sampled field window (the shared ViewTransform, ADR-0018). The
    // vignette below stays screen-anchored (uses unshifted `uv`).
    var p = uv * zoom + pan;
    for (var i = 0; i < 5; i = i + 1) {
        let fi = f32(i);
        p = p + warp * vec2<f32>(
            sin(p.y * 1.5 + t * 0.7 + fi),
            cos(p.x * 1.5 - t * 0.6 + fi)
        ) / (fi + 1.0);
    }

    let field = 0.5 + 0.5 * sin(p.x + p.y + t * 0.5);
    var col = palette(field * 0.6 + hue);

    let r = length(uv);
    col = col * (glow * (1.0 - 0.25 * r));
    col = col + vec3<f32>(flash * 0.12);

    return vec4<f32>(col, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Params {
    a: [f32; 4],
    b: [f32; 4],
    c: [f32; 4],
}

/// Fullscreen domain-warped fragment field, driven by named preset parameters.
pub struct FragmentFieldScene {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// Shared scene clock (seconds), set by the renderer each frame.
    time: f32,
    warp: f32,
    hue: f32,
    zoom: f32,
    glow: f32,
    flash: f32,
    pan_x: f32,
    pan_y: f32,
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
            warp: DEFAULT_WARP,
            hue: DEFAULT_HUE,
            zoom: DEFAULT_ZOOM,
            glow: DEFAULT_GLOW,
            flash: DEFAULT_FLASH,
            pan_x: DEFAULT_PAN,
            pan_y: DEFAULT_PAN,
        }
    }
}

impl Scene for FragmentFieldScene {
    fn name(&self) -> &'static str {
        "fragment field"
    }

    fn set_time(&mut self, time: f32) {
        self.time = time;
    }

    fn reset_params(&mut self) {
        self.warp = DEFAULT_WARP;
        self.hue = DEFAULT_HUE;
        self.zoom = DEFAULT_ZOOM;
        self.glow = DEFAULT_GLOW;
        self.flash = DEFAULT_FLASH;
        self.pan_x = DEFAULT_PAN;
        self.pan_y = DEFAULT_PAN;
    }

    fn set_param(&mut self, name: &str, value: f32) {
        match name {
            "warp" => self.warp = value,
            "hue" => self.hue = value,
            "zoom" => self.zoom = value,
            "glow" => self.glow = value,
            "flash" => self.flash = value,
            "pan_x" => self.pan_x = value,
            "pan_y" => self.pan_y = value,
            _ => {}
        }
    }

    fn update(&mut self, _frame: &AnalysisFrame) {
        // Fully parameter-driven; the analysis reaches this scene only through
        // the preset expressions bound to its parameters.
    }

    fn render(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        aspect: f32,
    ) {
        let params = Params {
            a: [self.time, aspect.max(0.1), self.warp, self.hue],
            b: [self.zoom, self.glow, self.flash, 0.0],
            c: [self.pan_x, self.pan_y, 0.0, 0.0],
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
