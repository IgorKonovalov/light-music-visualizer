//! The shared line primitive: a GPU helper that draws thick, glowing lines as
//! instanced camera-facing quads. Each [`SegmentInstance`] (two endpoints, a
//! colour, a half-width) is expanded in the vertex shader into a quad whose
//! width is uniform *on screen* — the swarm scene's instanced-quad pipeline
//! (ADR-0007) with segments in place of points. Additive blend, so overlapping
//! and dense strokes bloom.
//!
//! Native wgpu line primitives are deliberately not used: their width is locked
//! near 1px and varies by backend (ADR-0007). The buffer is fixed-capacity and
//! reused every frame, so a full curve upload never allocates on the hot path.

// Hot-path panic-denial pragma (Plan 0002 Phase 2, extended to scenes by Plan
// 0003 Phase 0). `draw` runs every displayed frame.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

/// One line segment: endpoints `a`/`b` in world space (x is divided by aspect
/// in the shader, matching the swarm's convention), an RGB colour, and a
/// half-width in NDC-y units (uniform on screen after the aspect divide).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct SegmentInstance {
    /// First endpoint (world space).
    pub a: [f32; 2],
    /// Second endpoint (world space).
    pub b: [f32; 2],
    /// RGB colour (pre-brightness; additive blend sums overlaps).
    pub color: [f32; 3],
    /// Half-width in NDC-y units.
    pub width: f32,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    // x: aspect, y: glow multiplier, zw: unused
    v: [f32; 4],
}

const SHADER: &str = r#"
struct Uniforms {
    v: vec4<f32>,
}

@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) side: f32,
    @location(1) color: vec3<f32>,
}

@vertex
fn vs_main(
    @builtin(vertex_index) vi: u32,
    @location(0) a: vec2<f32>,
    @location(1) b: vec2<f32>,
    @location(2) color: vec3<f32>,
    @location(3) width: f32,
) -> VsOut {
    // (along, side): along runs a->b, side spans -1..1 across the width.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[vi];
    let aspect = max(u.v.x, 0.1);
    let inv_aspect = 1.0 / aspect;

    // Work in aspect-corrected space so the perpendicular offset is a uniform
    // on-screen thickness whatever the segment's orientation.
    let a_s = vec2<f32>(a.x * inv_aspect, a.y);
    let b_s = vec2<f32>(b.x * inv_aspect, b.y);
    var dir = b_s - a_s;
    let len = length(dir);
    if (len > 1e-6) {
        dir = dir / len;
    } else {
        dir = vec2<f32>(1.0, 0.0);
    }
    let nrm = vec2<f32>(-dir.y, dir.x);
    let base = mix(a_s, b_s, c.x);
    let pos = base + nrm * c.y * width;

    var out: VsOut;
    out.pos = vec4<f32>(pos, 0.0, 1.0);
    out.side = c.y;
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Bright core, quadratic falloff to the quad edge: a soft glowing stroke.
    let d = abs(in.side);
    let falloff = max(0.0, 1.0 - d);
    let g = falloff * falloff;
    return vec4<f32>(in.color * g * u.v.y, 1.0);
}
"#;

/// Draws segment buffers as thick glowing quads. Owns its pipeline, a
/// fixed-capacity instance buffer, and the aspect/glow uniform.
pub struct LineRenderer {
    pipeline: wgpu::RenderPipeline,
    instances: wgpu::Buffer,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// Maximum segments the instance buffer holds; extra are dropped by `draw`.
    capacity: usize,
}

impl LineRenderer {
    /// Build the pipeline and a `capacity`-segment instance buffer on `device`.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        capacity: usize,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("line-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("line-instances"),
            size: (capacity * std::mem::size_of::<SegmentInstance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("line-uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("line-bind-layout"),
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
            label: Some("line-bind-group"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("line-pipeline-layout"),
            bind_group_layouts: &[Some(&bind_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("line-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<SegmentInstance>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2,
                        1 => Float32x2,
                        2 => Float32x3,
                        3 => Float32,
                    ],
                })],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    // Additive: overlapping strokes bloom brighter.
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

        Self {
            pipeline,
            instances,
            uniforms,
            bind_group,
            capacity,
        }
    }

    /// Segments the instance buffer can hold — the scene clamps its geometry to
    /// this and surfaces any drop at load (ADR-0007 cap must never be silent).
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Clear `view` to `clear` and draw `segments` as thick glowing quads at the
    /// given `aspect` and `glow` multiplier. Segments beyond [`capacity`] are
    /// dropped defensively (the scene is responsible for capping at load).
    #[allow(
        clippy::too_many_arguments,
        reason = "distinct GPU handles plus the per-frame draw parameters (aspect, glow, clear); \
                  bundling them would only shuffle the same values behind a one-use struct"
    )]
    pub fn draw(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        aspect: f32,
        glow: f32,
        clear: wgpu::Color,
        segments: &[SegmentInstance],
    ) {
        let count = segments.len().min(self.capacity);
        let drawn = segments.get(..count).unwrap_or(&[]);
        if !drawn.is_empty() {
            queue.write_buffer(&self.instances, 0, bytemuck::cast_slice(drawn));
        }
        queue.write_buffer(
            &self.uniforms,
            0,
            bytemuck::bytes_of(&Uniforms {
                v: [aspect.max(0.1), glow, 0.0, 0.0],
            }),
        );

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("line-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if drawn.is_empty() {
            return; // cleared the frame; nothing to stroke
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.instances.slice(..));
        pass.draw(0..6, 0..drawn.len() as u32);
    }
}
