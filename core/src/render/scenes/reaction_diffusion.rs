//! Reaction-diffusion scene: a Gray-Scott simulation evolving on a fixed
//! internal grid via the reusable [`PingPongField`](crate::render::feedback)
//! (ADR-0012). The engine's first *stateful* scene — each frame's field depends
//! on the previous frame's, held in a texture and stepped by a simulation
//! shader — unlocking the organic, restructuring look (nested contours, cellular
//! tissue, a hatched maze) stateless scenes can't produce.
//!
//! Phase 1 is a walking skeleton: a fixed number of sub-steps per frame and a
//! grayscale present. The fixed-timestep accumulator (Phase 2), audio-reactive
//! named parameters (Phase 3), and the iso-contour/hatch look (Phase 4) land on
//! top of this. All randomness is the seeded initial scatter (NFR 6): the field
//! is a pure function of the seed + the fixed-`dt` step sequence.
//!
//! **GPU resources are built lazily, on first render.** The scene stores a
//! device handle and constructs its pipelines/textures only when it is first
//! drawn (see [`Resources`]). This keeps the resources off the device until the
//! scene is actually shown, and — importantly — lets the headless capture tests
//! build the full roster on the DX12 WARP software adapter: WARP mis-renders the
//! pre-existing fragment-field pipeline once this scene's *full* set of feedback
//! resources coexists on the device (a cumulative software-rasterizer quirk with
//! no wgpu validation error; real hardware is unaffected). Deferring
//! construction means a capture that never activates this scene never builds
//! those resources, so the other scenes' captures stay correct; a capture that
//! *does* activate it builds them and renders this scene normally.

// Hot-path panic-denial pragma (Plan 0002 Phase 2, extended to scenes by Plan
// 0003 Phase 0). Encodes its passes every displayed frame.
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

/// Fixed internal simulation grid (square). 256² resolves the Gray-Scott
/// patterns well while staying cheap enough that the headless capture tests run
/// briskly on the software (WARP) adapter — a 512² grid quadruples the per-step
/// fragment work the differential tests pay each warm-up frame (ADR-0012 gives
/// 512² only as an example).
const GRID: u32 = 256;

/// Wall-clock duration of one Gray-Scott sub-step (Plan 0014 Phase 2). The
/// fixed-timestep accumulator runs one sub-step per `FIXED_STEP` of injected
/// real `dt`, so the simulation evolves at the same rate on any refresh — at the
/// live/​capture `dt` of 1/60 s this is 12 sub-steps per frame.
const FIXED_STEP: f32 = 1.0 / 720.0;

/// Max sub-steps encoded in a single frame. A long stall would otherwise queue
/// unbounded work (accumulator spiral-of-death); past this the accumulator's
/// backlog is dropped, so the sim briefly slows rather than diverging (ADR-0012).
/// 40 covers a ~55 ms hitch before it bites.
const MAX_SUBSTEPS: u32 = 40;

/// Seeded initial-scatter blobs, and the uniform array's capacity.
const SEED_BLOBS: usize = 30;
const MAX_BLOBS: usize = 32;
const SEED: u64 = 0x4C4D_565F_5244_5F31; // "LMV_RD_1"

/// Parameter defaults — the "mitosis" Gray-Scott regime (Pearson's
/// classification): spots that perpetually divide, so the field keeps
/// restructuring rather than settling into a static pattern.
const DEFAULT_FEED: f32 = 0.0367;
const DEFAULT_KILL: f32 = 0.0649;
/// Diffusion rates for the two species (classic Karl Sims values at internal
/// `dt = 1`, paired with the 3×3 Laplacian kernel in the shader). The `flow`
/// param (Phase 3) scales both, keeping their ratio, so a band can coarsen or
/// tighten the pattern's spatial scale.
const DIFFUSE_U: f32 = 0.16;
const DIFFUSE_V: f32 = 0.08;
/// `flow` default: unscaled diffusion.
const DEFAULT_FLOW: f32 = 1.0;

/// Present-look defaults (Phase 4): palette hue offset, iso-contour band count,
/// hatch stripe spacing in texels, and glow strength.
const DEFAULT_HUE: f32 = 0.0;
const DEFAULT_CONTOUR: f32 = 6.0;
const DEFAULT_HATCH: f32 = 5.0;
const DEFAULT_GLOW: f32 = 1.0;

/// Beat-stamped seed injection (Phase 3). A rising `inject` edge stamps a blob
/// of V into the field at the next seeded position, so a beat spawns new growth.
/// Positions come from a `SeededRng` (NFR 6) so a capture reproduces exactly.
const INJECT_RADIUS: f32 = 0.045;
const INJECT_AMOUNT: f32 = 0.85;
const INJECT_SEED: u64 = 0x4C4D_5244_494E_4A31; // "LMRDINJ1"
/// `inject` rises past this to fire one stamp (edge-triggered, not per-frame).
const INJECT_THRESHOLD: f32 = 0.5;

/// Seed pass: U = 1 everywhere, V = 1 inside the scattered blobs.
const INIT_SHADER: &str = r#"
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

struct Init {
    blobs: array<vec4<f32>, 32>, // xy: center (uv), z: radius, w: unused
    count: vec4<u32>,            // x: active blob count
}
@group(0) @binding(0) var<uniform> init: Init;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var v = 0.0;
    let n = init.count.x;
    for (var i = 0u; i < n; i = i + 1u) {
        let b = init.blobs[i];
        if (distance(in.uv, b.xy) < b.z) {
            v = 1.0;
        }
    }
    return vec4<f32>(1.0 - v, v, 0.0, 1.0);
}
"#;

/// Sim pass: one Gray-Scott step, reading the previous field, writing the next.
const SIM_SHADER: &str = r#"
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

struct Sim {
    p: vec4<f32>,   // x: feed, y: kill, z: diffuse_u, w: diffuse_v
    inj: vec4<f32>, // xy: stamp center (uv), z: radius, w: amount (0 = no stamp)
}
@group(0) @binding(0) var<uniform> sim: Sim;
@group(0) @binding(1) var field: texture_2d<f32>;

// Toroidal texel fetch (wrap at the edges) of the (U, V) pair.
fn ld(c: vec2<i32>, size: vec2<i32>) -> vec2<f32> {
    let w = ((c % size) + size) % size;
    return textureLoad(field, w, 0).xy;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(field));
    let c = vec2<i32>(i32(in.pos.x), i32(in.pos.y));
    let m = ld(c, size);
    let u = m.x;
    let v = m.y;

    // 3×3 Laplacian: orthogonal 0.2, diagonal 0.05, center -1.
    var lap = ld(c + vec2<i32>(-1, 0), size) * 0.2
        + ld(c + vec2<i32>(1, 0), size) * 0.2
        + ld(c + vec2<i32>(0, -1), size) * 0.2
        + ld(c + vec2<i32>(0, 1), size) * 0.2
        + ld(c + vec2<i32>(-1, -1), size) * 0.05
        + ld(c + vec2<i32>(1, -1), size) * 0.05
        + ld(c + vec2<i32>(-1, 1), size) * 0.05
        + ld(c + vec2<i32>(1, 1), size) * 0.05;
    lap = lap - m;

    let feed = sim.p.x;
    let kill = sim.p.y;
    let du = sim.p.z;
    let dv = sim.p.w;
    let reaction = u * v * v;
    let nu = u + du * lap.x - reaction + feed * (1.0 - u);
    var nv = v + dv * lap.y + reaction - (kill + feed) * v;

    // Beat-stamped seed injection (Phase 3), folded into the sim so no extra
    // pipeline is needed. `inj.w` is non-zero only on the stamp frame; it is
    // applied on every sub-step of that frame, so V saturates at the stamp.
    let stamp = sim.inj.w * (1.0 - smoothstep(sim.inj.z * 0.4, sim.inj.z, distance(in.uv, sim.inj.xy)));
    nv = nv + stamp;

    return vec4<f32>(clamp(nu, 0.0, 1.0), clamp(nv, 0.0, 1.0), 0.0, 1.0);
}
"#;

/// Present pass (Phase 4): the reference aesthetic — analytic iso-contours of
/// the V field with `fwidth` anti-aliasing, a cosine palette coloring the nested
/// loops, gradient-aligned hatch/comb ticks, and a soft glow.
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

struct Present {
    // x: hue, y: contour density, z: hatch frequency (texels), w: glow
    a: vec4<f32>,
}
@group(0) @binding(0) var present_field: texture_2d<f32>;
@group(0) @binding(1) var present_samp: sampler;
@group(0) @binding(2) var<uniform> pp: Present;

// iq-style cosine palette: smooth, loops in hue.
fn palette(t: f32) -> vec3<f32> {
    let a = vec3<f32>(0.5, 0.5, 0.5);
    let b = vec3<f32>(0.5, 0.5, 0.5);
    let c = vec3<f32>(1.0, 1.0, 1.0);
    let d = vec3<f32>(0.0, 0.33, 0.67);
    return a + b * cos(6.28318 * (c * t + d));
}

fn sample_v(uv: vec2<f32>) -> f32 {
    return textureSampleLevel(present_field, present_samp, uv, 0.0).y;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(present_field));
    let texel = 1.0 / dims;
    let uv = in.uv;

    let v = sample_v(uv);

    // Central-difference gradient of the field (for hatch orientation + edges).
    let gx = sample_v(uv + vec2<f32>(texel.x, 0.0)) - sample_v(uv - vec2<f32>(texel.x, 0.0));
    let gy = sample_v(uv + vec2<f32>(0.0, texel.y)) - sample_v(uv - vec2<f32>(0.0, texel.y));
    let grad = vec2<f32>(gx, gy);
    let gmag = length(grad);

    let hue = pp.a.x;
    let density = pp.a.y;
    let hatch_freq = pp.a.z;
    let glow = pp.a.w;

    // Slope mask: contours and hatch only appear where the field actually
    // slopes, so the flat V=0 background stays dark (V=0 is itself an iso-level,
    // which would otherwise flood the flats).
    let slope = smoothstep(0.0008, 0.004, gmag);

    // Iso-contour lines: distance (in pixels) to the nearest V = k/density level,
    // anti-aliased by fwidth. `contour` is ~1 on a line, 0 between them.
    let f = v * density;
    let line_d = abs(fract(f - 0.5) - 0.5) / max(fwidth(f), 1e-4);
    let contour = (1.0 - clamp(line_d, 0.0, 1.0)) * slope;

    // Palette by field level so the nested loops read as coloured bands.
    let col = palette(v * 0.85 + hue);

    // Hatch/comb: stripes along the contour tangent (perpendicular to grad),
    // gated to the slopes so flats stay clean.
    let tang = normalize(vec2<f32>(-grad.y, grad.x) + vec2<f32>(1e-5, 1e-5));
    let s = dot(uv * dims, tang) / max(hatch_freq, 1.0);
    let hatch = smoothstep(0.30, 0.5, abs(fract(s) - 0.5));
    let hatch_amt = hatch * slope;

    // Compose: dark bed, a coloured fill only where the field lives, bright
    // contour loops, hatch ticks that darken along the slopes, and a soft glow.
    let structure = smoothstep(0.04, 0.45, v);
    var out_col = col * structure * 0.5;
    out_col = out_col + col * contour * 0.9;
    out_col = out_col * (1.0 - hatch_amt * 0.4);
    out_col = out_col + col * v * glow * 0.22;

    return vec4<f32>(out_col, 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct InitParams {
    blobs: [[f32; 4]; MAX_BLOBS],
    count: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct SimParams {
    /// x: feed, y: kill, z: diffuse_u, w: diffuse_v.
    p: [f32; 4],
    /// xy: injection stamp center (uv), z: radius, w: amount (0 = none).
    inj: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PresentParams {
    /// x: hue, y: contour density, z: hatch frequency (texels), w: glow.
    a: [f32; 4],
}

/// The GPU-side state, built lazily on first render (see the module docs).
struct Resources {
    field: PingPongField,
    sim_pipeline: wgpu::RenderPipeline,
    init_pipeline: wgpu::RenderPipeline,
    present_pipeline: wgpu::RenderPipeline,
    sim_uniform: wgpu::Buffer,
    init_uniform: wgpu::Buffer,
    present_uniform: wgpu::Buffer,
    /// Sim/present bind groups reading texture A / texture B — selected by the
    /// field's read side each sub-step so nothing is rebuilt on the hot path.
    sim_bg_a: wgpu::BindGroup,
    sim_bg_b: wgpu::BindGroup,
    init_bg: wgpu::BindGroup,
    present_bg_a: wgpu::BindGroup,
    present_bg_b: wgpu::BindGroup,
}

impl Resources {
    /// Create every pipeline, buffer, bind group, and the ping-pong field.
    fn build(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let init_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rd-init-shader"),
            source: wgpu::ShaderSource::Wgsl(INIT_SHADER.into()),
        });
        let sim_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rd-sim-shader"),
            source: wgpu::ShaderSource::Wgsl(SIM_SHADER.into()),
        });
        let present_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("rd-present-shader"),
            source: wgpu::ShaderSource::Wgsl(PRESENT_SHADER.into()),
        });

        let field = PingPongField::new(device, GRID, GRID);

        let sim_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rd-sim-params"),
            size: std::mem::size_of::<SimParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let init_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rd-init-params"),
            size: std::mem::size_of::<InitParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let present_uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rd-present-params"),
            size: std::mem::size_of::<PresentParams>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        // --- init pipeline: one uniform, writes the seed field ---
        let init_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rd-init-layout"),
            entries: &[uniform_entry(0)],
        });
        let init_bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rd-init-bg"),
            layout: &init_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: init_uniform.as_entire_binding(),
            }],
        });
        let init_pipeline = field_pipeline(device, &init_shader, &init_layout, "rd-init");

        // --- sim pipeline: uniform + input texture (textureLoad, no sampler) ---
        let sim_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rd-sim-layout"),
            entries: &[uniform_entry(0), texture_entry(1, false)],
        });
        let sim_bg_a = sim_bind_group(device, &sim_layout, &sim_uniform, field.view_a());
        let sim_bg_b = sim_bind_group(device, &sim_layout, &sim_uniform, field.view_b());
        let sim_pipeline = field_pipeline(device, &sim_shader, &sim_layout, "rd-sim");

        // --- present pipeline: input texture + filtering sampler, to surface ---
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("rd-present-sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let present_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rd-present-layout"),
            entries: &[texture_entry(0, true), sampler_entry(1), uniform_entry(2)],
        });
        let present_bg_a = present_bind_group(
            device,
            &present_layout,
            field.view_a(),
            &sampler,
            &present_uniform,
        );
        let present_bg_b = present_bind_group(
            device,
            &present_layout,
            field.view_b(),
            &sampler,
            &present_uniform,
        );
        let present_pipeline =
            surface_pipeline(device, &present_shader, &present_layout, surface_format);

        Self {
            field,
            sim_pipeline,
            init_pipeline,
            present_pipeline,
            sim_uniform,
            init_uniform,
            present_uniform,
            sim_bg_a,
            sim_bg_b,
            init_bg,
            present_bg_a,
            present_bg_b,
        }
    }

    /// Encode the one-shot seed pass into the current read texture, filling the
    /// field with the deterministic initial pattern. Run once after a (re)build.
    fn encode_seed(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("rd-seed-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: self.field.read_view(),
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
        pass.set_pipeline(&self.init_pipeline);
        pass.set_bind_group(0, &self.init_bg, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Gray-Scott reaction-diffusion on a ping-pong field, driven by named preset
/// parameters (ADR-0002 layer 2): `feed`/`kill` pick the regime, `flow` scales
/// the diffusion, and a rising `inject` edge stamps a seeded blob of growth.
pub struct ReactionDiffusionScene {
    /// Cloned device handle (an `Arc` inside wgpu) used to build [`Resources`]
    /// lazily on first render — see the module docs for why.
    device: wgpu::Device,
    surface_format: wgpu::TextureFormat,
    res: Option<Resources>,
    /// The deterministic seed pattern, uploaded on the first frame after a
    /// (re)build so a rebuilt scene restarts identically (capture determinism).
    init_params: InitParams,
    needs_seed: bool,
    /// Fixed-timestep accumulator: unspent injected `dt`, drained one
    /// [`FIXED_STEP`] at a time in [`advance`](Scene::advance).
    accumulator: f32,
    /// Sub-steps `advance` scheduled for the next `render` to encode.
    pending_substeps: u32,
    /// Seeded RNG for injection stamp positions (NFR 6); advanced only when a
    /// stamp fires, and reset with the scene so a capture reproduces exactly.
    stamp_rng: SeededRng,
    /// A stamp scheduled by an `inject` rising edge for the next `render`:
    /// (cx, cy, radius, amount). `None` when no beat fired this frame.
    pending_stamp: Option<[f32; 4]>,
    /// Previous frame's `inject` value, for rising-edge detection.
    prev_inject: f32,
    /// Shared scene clock (seconds), set by the renderer each frame.
    time: f32,
    feed: f32,
    kill: f32,
    /// Diffusion scale (multiplies both species' rates, keeping their ratio).
    flow: f32,
    /// This frame's injection level (bound to a beat/onset expression).
    inject: f32,
    /// Present-look params (Phase 4): palette hue, iso-contour density, hatch
    /// stripe spacing, glow strength.
    hue: f32,
    contour: f32,
    hatch: f32,
    glow: f32,
}

impl ReactionDiffusionScene {
    /// Build the CPU-side state and compute the deterministic seed pattern. GPU
    /// resources are deferred to the first render (module docs).
    pub fn new(device: &wgpu::Device, surface_format: wgpu::TextureFormat) -> Self {
        let mut init_params = InitParams {
            blobs: [[0.0; 4]; MAX_BLOBS],
            count: [0; 4],
        };
        let mut rng = SeededRng::new(SEED);
        let n = SEED_BLOBS.min(MAX_BLOBS);
        for slot in init_params.blobs.iter_mut().take(n) {
            let x = rng.next_f32();
            let y = rng.next_f32();
            let r = rng.range(0.02, 0.045);
            *slot = [x, y, r, 0.0];
        }
        init_params.count = [n as u32, 0, 0, 0];

        Self {
            device: device.clone(),
            surface_format,
            res: None,
            init_params,
            needs_seed: true,
            accumulator: 0.0,
            pending_substeps: 0,
            stamp_rng: SeededRng::new(INJECT_SEED),
            pending_stamp: None,
            prev_inject: 0.0,
            time: 0.0,
            feed: DEFAULT_FEED,
            kill: DEFAULT_KILL,
            flow: DEFAULT_FLOW,
            inject: 0.0,
            hue: DEFAULT_HUE,
            contour: DEFAULT_CONTOUR,
            hatch: DEFAULT_HATCH,
            glow: DEFAULT_GLOW,
        }
    }
}

/// A fragment-visible uniform-buffer bind group layout entry.
fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// A fragment-visible sampled-texture layout entry. `filterable` must match how
/// the shader uses it: the sim reads via `textureLoad` (unfiltered), the present
/// pass via a filtering sampler.
fn texture_entry(binding: u32, filterable: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable },
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

fn sim_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform: &wgpu::Buffer,
    input: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("rd-sim-bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(input),
            },
        ],
    })
}

fn present_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    input: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    uniform: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("rd-present-bg"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(input),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: uniform.as_entire_binding(),
            },
        ],
    })
}

/// A fullscreen-triangle pipeline writing into the [`PingPongField::FORMAT`]
/// grid (the init and sim passes).
fn field_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    bind_layout: &wgpu::BindGroupLayout,
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
                format: PingPongField::FORMAT,
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

/// The present pipeline: fullscreen triangle to the surface format.
fn surface_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    bind_layout: &wgpu::BindGroupLayout,
    surface_format: wgpu::TextureFormat,
) -> wgpu::RenderPipeline {
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("rd-present"),
        bind_group_layouts: &[Some(bind_layout)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("rd-present-pipeline"),
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
    })
}

impl Scene for ReactionDiffusionScene {
    fn name(&self) -> &'static str {
        "reaction diffusion"
    }

    fn advance(&mut self, dt: f32) {
        // Drain the accumulator one fixed sub-step at a time, clamped so a long
        // stall can't queue unbounded work (ADR-0012). The sub-`FIXED_STEP`
        // remainder carries to the next frame; a clamp drops the excess backlog
        // so the sim slows rather than races to catch up.
        self.accumulator += dt;
        let mut steps = 0u32;
        while self.accumulator >= FIXED_STEP && steps < MAX_SUBSTEPS {
            self.accumulator -= FIXED_STEP;
            steps += 1;
        }
        self.accumulator = self.accumulator.min(FIXED_STEP);
        self.pending_substeps = steps;
    }

    fn set_time(&mut self, time: f32) {
        self.time = time;
    }

    fn reset_params(&mut self) {
        self.feed = DEFAULT_FEED;
        self.kill = DEFAULT_KILL;
        self.flow = DEFAULT_FLOW;
        self.inject = 0.0;
        self.hue = DEFAULT_HUE;
        self.contour = DEFAULT_CONTOUR;
        self.hatch = DEFAULT_HATCH;
        self.glow = DEFAULT_GLOW;
    }

    fn set_param(&mut self, name: &str, value: f32) {
        // ADR-0002 layer 2 knobs. `feed`/`kill` pick the regime; `flow` scales
        // the diffusion; `inject` is a beat/onset level whose rising edge stamps
        // a seed (edge detected in `update`). `hue`/`contour`/`hatch`/`glow`
        // drive the iso-contour present look (Phase 4).
        match name {
            "feed" => self.feed = value,
            "kill" => self.kill = value,
            "flow" => self.flow = value,
            "inject" => self.inject = value,
            "hue" => self.hue = value,
            "contour" => self.contour = value,
            "hatch" => self.hatch = value,
            "glow" => self.glow = value,
            _ => {}
        }
    }

    fn update(&mut self, _frame: &AnalysisFrame) {
        // Rising-edge detect on `inject` (a beat/onset expression): schedule one
        // stamp at the next seeded position. Edge-triggered so a sustained beat
        // flag doesn't stamp every frame; deterministic because the position
        // comes from the seeded, capture-reset `stamp_rng` (NFR 6).
        if self.inject >= INJECT_THRESHOLD && self.prev_inject < INJECT_THRESHOLD {
            let cx = self.stamp_rng.next_f32();
            let cy = self.stamp_rng.next_f32();
            self.pending_stamp = Some([cx, cy, INJECT_RADIUS, INJECT_AMOUNT]);
        }
        self.prev_inject = self.inject;
    }

    fn render(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        _aspect: f32,
    ) {
        // Build GPU resources on first use (module docs).
        if self.res.is_none() {
            self.res = Some(Resources::build(&self.device, self.surface_format));
        }
        let Self {
            res,
            init_params,
            needs_seed,
            pending_substeps,
            pending_stamp,
            feed,
            kill,
            flow,
            hue,
            contour,
            hatch,
            glow,
            ..
        } = self;
        let Some(res) = res.as_mut() else {
            return;
        };

        queue.write_buffer(
            &res.present_uniform,
            0,
            bytemuck::bytes_of(&PresentParams {
                a: [*hue, *contour, *hatch, *glow],
            }),
        );

        // A beat scheduled a stamp this frame (consumed here): the sim shader
        // applies it on every sub-step, so V saturates at the stamp. `[0; 4]`
        // means no injection.
        let inj = pending_stamp.take().unwrap_or([0.0; 4]);
        queue.write_buffer(
            &res.sim_uniform,
            0,
            bytemuck::bytes_of(&SimParams {
                // `flow` scales both diffusion rates, keeping the 2:1 ratio (so
                // the pattern coarsens/tightens without changing regime).
                p: [*feed, *kill, DIFFUSE_U * *flow, DIFFUSE_V * *flow],
                inj,
            }),
        );

        // One-shot deterministic seed on the first frame after a (re)build.
        if *needs_seed {
            queue.write_buffer(&res.init_uniform, 0, bytemuck::bytes_of(init_params));
            res.encode_seed(encoder);
            *needs_seed = false;
        }

        // Run the sub-steps the accumulator scheduled this frame (Phase 2). Each
        // reads the current field and writes the other texture, then swaps.
        for _ in 0..*pending_substeps {
            let sim_bg = if res.field.reading_a() {
                &res.sim_bg_a
            } else {
                &res.sim_bg_b
            };
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("rd-sim-pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: res.field.write_view(),
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            // The fullscreen sim pass overwrites every texel.
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&res.sim_pipeline);
                pass.set_bind_group(0, sim_bg, &[]);
                pass.draw(0..3, 0..1);
            }
            res.field.swap();
        }

        // Present the latest field to the surface.
        let present_bg = if res.field.reading_a() {
            &res.present_bg_a
        } else {
            &res.present_bg_b
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("rd-present-pass"),
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
