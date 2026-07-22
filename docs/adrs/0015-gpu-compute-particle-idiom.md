# ADR-0015 — GPU compute pipelines for particle scenes; the four render-idiom catalogue

> **Status:** proposed
> **Date:** 2026-07-22
> **Related plan(s):** [0016-gpu-compute-particle-scenes](../plans/0016-gpu-compute-particle-scenes.md)

## Context

A survey of generative-art techniques worth adding as new scene families (strange
attractors, curl-noise flow fields, fractal flames, reaction-diffusion, Lenia, Chladni
plates, raymarched SDFs, superformula, walkers) collapses onto **four GPU render
idioms** — and three of the four already exist or are already designed in this repo:

| Idiom | Families it hosts | State in this repo |
|-------|-------------------|--------------------|
| **A. Line/point strips** | parametric curves, superformula, harmonograph, epicycloid, L-systems | **Exists / in progress** — `render/scenes/lines/` + Plan 0010 (`LineRenderer`, parametric + generator systems). New curves are content on this system. |
| **C. Texture-feedback ping-pong** | Gray-Scott reaction-diffusion, Lenia, walker trails | **Designed** — Plan 0014 + [ADR-0012](0012-stateful-feedback-render-system.md) build `render::feedback::PingPongField`. |
| **D. Full-screen fragment** | Chladni, raymarched SDF, MilkDrop-style fields | **Exists** — `render/scenes/fragment_field.rs` is a Shadertoy-style fullscreen field. New looks are shaders on this pattern. |
| **B. GPU particles** | strange attractors, curl-noise flow fields, fractal flames, boids | **Gap.** `render/scenes/swarm.rs` is a *CPU* ~10k-point swarm; a compute path is a deferred Plan 0003 follow-up. |

Idiom B is the one genuinely-missing capability. The verified way to render the
attractor/particle family at its signature scale (tens to hundreds of thousands of
glowing points with trails) keeps **all particle state resident on the GPU** — a
storage buffer (or state texture) advanced each frame, never read back to the CPU. Two
framings were explicitly *refuted* during research: attractors are **not** best drawn as
CPU-integrated polylines, and **not** as a per-particle RK4 ODE integrator — the point
cloud is the artifact, and the map/step is a cheap per-particle kernel.

The existing CPU swarm caps at ~10k because every point is integrated on the render
thread each frame; pushing it to the particle-family scale would burn CPU against the
project's "lightweight / low idle" value (CLAUDE.md) and still miss the dense look. The
decision is therefore *how* to add GPU-resident particle stepping: a real compute
pipeline, or a fragment/texture-state trick that reuses idiom C's ping-pong plumbing.

The core has **no compute pipeline today** — every current scene is vertex+fragment. So
adopting compute is a genuine new-capability decision, not a casual edit: it adds a
shader stage and a `wgpu::ComputePipeline` binding path to the engine, and its
integrated-GPU cost profile (compute dispatch + additive-blend fill rate) is unproven on
our low-end target. That is what makes this ADR-worthy.

## Decision

We will add **idiom B as a GPU compute-pipeline particle system in `core`**: particle
state (position, age, seed) lives in a `wgpu` storage buffer, a **compute shader** steps
every particle each frame from an injected real `dt` (the frame-rate-independent clock
Plan 0014 establishes by retiring `SCENE_DT`), and a render pass draws the particles as
additive point-sprites, with trails produced by a fade/feedback pass (reusing Plan
0014's `PingPongField` idiom rather than inventing a second one). Particle-look scalars
(attractor coefficients, point size, hue, trail fade) are exposed through ADR-0002's
existing layer-2 named-parameter surface, so audio reactivity flows through the preset
expression layer exactly as it does for the fragment and swarm systems — no new audio
plumbing and no C ABI change (both frontends render core scenes over the frozen surface,
so the foobar plugin inherits the new scenes for free). Visual randomness stays
explicitly seeded (`SeededRng`) so a scene is reproducible (NFR §6). Plan 0016 lands this
with strange attractors as the first family. We rejected the **fragment/texture-state**
approach because it binds particle count to texture dimensions, adds vertex-pull
indirection, and does not generalize cleanly to fractal flames' density-histogram
accumulation; and the **CPU-swarm extension** because its ~10k ceiling and per-frame
render-thread cost defeat both the scale and the lightweight goals.

## Consequences

### Positive
- Unlocks a whole family on one path: after attractors, curl-noise flow fields, fractal
  flames, and boids are all "same compute step, different kernel."
- Particle state is GPU-resident — no CPU round-trip, low idle CPU, scales to 100k+
  points at ~16 bytes each (negligible memory; ~1.6 MB at 100k).
- Reuses Plan 0014's `PingPongField` for trails instead of a second feedback mechanism —
  the two stateful idioms (feedback sims, particles) share one fade primitive.
- Audio reactivity and determinism come for free from the existing ADR-0002 param layer
  and `SeededRng`; the capture/QA harness (Plan 0013) can test the new scene like any other.

### Negative
- **First compute pipeline in `core`** — a new engine capability surface (compute shader
  stage, storage-buffer bind groups, dispatch sizing) that every future particle family
  and the hygiene/panic-pragma guard must account for.
- **Integrated-GPU cost is unproven.** The real iGPU risk is not memory but additive
  **overdraw fill rate** at high point counts, plus compute-dispatch overhead. Needs an
  on-device smoke (routes to `docs/on-device-validation.md`, non-blocking) and a
  particle-count knob so a preset can stay within the 60 fps floor (NFR §1).
- **Cross-vendor floating-point divergence is amplified by chaos.** Attractor iteration
  is sensitive to FP rounding, so pixel-exact golden images will differ across GPU
  adapters. Golden regression must lean on the software-adapter baseline + tolerance
  (as Plan 0013 already does) or assert structural metrics (coverage/spread), not exact pixels.

### Neutral
- The engine's named render idioms grow from three to four; scene code still writes to
  the wgpu abstraction only (ADR-0001) — no per-backend branching.
- The `Scene` trait may gain a particle-family selector via the existing ADR-0007
  `configure` hook rather than a new trait method — keeping the seam thin.

## Alternatives considered

### Alternative A — Fragment/texture-state particles (reuse idiom C ping-pong)
Store particle state in a texture, iterate the attractor map in a fragment shader
(ping-pong), and draw points by vertex-pulling positions from the state texture. It adds
**no** new pipeline kind and rides Plan 0014's plumbing. Rejected because particle count
is bound to texture dimensions, the vertex-pull-from-texture path is more indirection for
no scale benefit over a storage buffer, and it does not extend to fractal flames — whose
log-density **histogram accumulation** wants a compute/atomic path anyway, meaning we'd
pay for compute later regardless.

### Alternative B — Extend the existing CPU swarm
Integrate attractor ODEs on the CPU into the existing ~10k-point swarm vertex buffer each
frame. Zero new render infra and the simplest possible change. Rejected because the ~10k
ceiling misses the signature dense-particle look, and integrating every particle on the
render thread each frame burns CPU against the lightweight/low-idle value — the exact cost
GPU-resident state exists to avoid.

## Notes

- Idiom framing and the attractor rendering facts (GPU-resident state; refuted CPU-polyline
  and per-particle-RK4 framings; 16-byte particles scaling to 100M in glChAoS.P) come from
  the 2026-07-22 deep-research pass on generative techniques for the visualizer.
- Curl-noise flow fields, fractal flames (histogram + log tonemap), and boids are deliberately
  **not** in the first plan — they are follow-ups on this same compute idiom, tracked in Plan 0016.
