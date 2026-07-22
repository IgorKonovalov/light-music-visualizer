# Generative-technique catalogue (scene-family backlog)

A reference list, not a plan. It catalogues generative / algorithmic / procedural art
techniques worth adding as new scene families, organised by the **GPU render idiom** each
needs, with an audio-reactivity and implementation-cost note per technique. Source: the
2026-07-22 deep-research pass (fan-out web search + adversarial verification) plus a focused
pass on walkers and *The Nature of Code*. Design decisions derived from this live in
[ADR-0015](adrs/0015-gpu-compute-particle-idiom.md); the first plan it drives is
[Plan 0016](plans/0016-gpu-compute-particle-scenes.md).

Treat this as the backlog a future architect session shops from — pick a technique, check
which idiom it needs, see whether that idiom exists yet, then interview + plan.

## The four render idioms (and what exists here)

Almost every technique below collapses onto one of four GPU idioms. Building an idiom once
unlocks its whole family, so the idiom — not the individual technique — is the unit of work.

| Idiom | What the engine needs | Repo status |
|-------|-----------------------|-------------|
| **A. Line / point strips** | vertex buffer of evaluated points (`LineRenderer`) | **Exists / in progress** — `render/scenes/lines/`, Plan 0010 |
| **B. GPU particles** | storage buffer + compute step + additive/instanced draw | **In progress via [Plan 0016](plans/0016-gpu-compute-particle-scenes.md)** — the first compute pipeline ([ADR-0015](adrs/0015-gpu-compute-particle-idiom.md)) |
| **C. Texture-feedback ping-pong** | two offscreen textures, read one / write other, fade | **Designed** — Plan 0014 + [ADR-0012](adrs/0012-stateful-feedback-render-system.md) (`PingPongField`) |
| **D. Full-screen fragment** | one quad, all colour in the pixel shader | **Exists** — `render/scenes/fragment_field.rs` |

Two facts that shape everything:

- **We already have a stronger audio hook than the "standard" recipe.** Real-world visualizers
  (MilkDrop/projectM) feed a handful of fixed uniforms (bass/mid/treble/beat) into shaders. This
  project's ADR-0002 layer-2 binds *arbitrary named parameters* to *expressions over the full
  analysis frame*. So idiom-D scenes need new **shaders**, never new audio plumbing.
- **Techniques with a small scalar parameter set are the most audio-modulatable** — superformula
  `m/n1/n2/n3`, Gray-Scott `F/K`, Lenia `mu/sigma`, attractor coefficients `a,b,c,d`. A few knobs
  routed onto bands/beat is the whole reactivity story.

---

## Idiom A — line / point strips (have it: `lines/`, Plan 0010)

Cheap, plotter-clean, rides the existing `LineRenderer`. New entries here are mostly **content**
(a new curve family + presets), not new infrastructure.

| Technique | What it is | Audio fit | Cost |
|-----------|-----------|-----------|------|
| **Superformula / supershapes** | Gielis generalisation of the superellipse; `m` symmetry + `n1/n2/n3` exponents → huge range of organic 2D/3D forms | High — exponents morph the whole shape | Trivial (new parametric family) |
| **Harmonograph** | sum of decaying sinusoids (simulated pendulums) → looping Lissajous-like figures | High — frequencies/phases/decay from bands | Trivial |
| **Epicycloid / hypotrochoid (Fourier drawing)** | point traced by circles rolling on circles; the spirograph / Fourier-series curve | High — radii/ratios from spectrum | Trivial |
| **L-systems / fractal branching** | rewriting grammar → turtle geometry (trees, ferns, Koch) | Med — branch angle → centroid, depth → energy, beat → regrow | Moderate (CPU-generate + cache; already in Plan 0010) |

Research note: superformula/harmonograph/epicycloid did **not** get a verified source in the
main pass, but they are textbook math and near-trivial extensions of the existing curve system —
absence of a citation reflects the verified-claim set, not any doubt about feasibility.

## Idiom B — GPU particles (building now: Plan 0016)

The genuine new capability. Compute shader steps particle state in a storage buffer; additive
point-sprite draw; trails via a fade pass. Verified: state stays GPU-resident (no CPU round-trip);
attractors scale to 100M points at ~16 bytes each. Refuted framings: **not** CPU polylines, **not**
a per-particle RK4 integrator.

| Technique | What it is | Audio fit | Cost |
|-----------|-----------|-----------|------|
| **Strange attractors** (De Jong, Clifford, Thomas, Lorenz, Aizawa) | iterate a chaotic map/ODE per particle; the point cloud is the artifact | **High** — coefficients `a,b,c,d`; beat → reseed/kick | Cheapest high-impact particle family — **Plan 0016** |
| **Curl-noise flow fields** | advect particles by curl of a noise field (divergence-free → swirly, incompressible) | **High** — noise scale + advection strength | Moderate (same compute path) |
| **Fractal flames** (Electric Sheep / Apophysis) | IFS "chaos game": random non-linear *variation*, accumulate into a **log-density histogram**, log-tonemap | **High** — variation weights + affine coeffs | Moderate-heavy (needs histogram/atomic + tonemap pass) |
| **Boids / flocking** | Reynolds steering: separation + alignment + cohesion over neighbours | High — neighbour radius → centroid, speed → amplitude | Moderate (needs spatial-hash grid to stay O(n)) |
| **Particle systems** (emitter/lifespan) | beat → emission burst, amplitude → rate/velocity, band → colour | **Very high** (the archetypal visualizer scene) | Moderate (GPU particle buffer + additive) |

## Idiom C — texture-feedback ping-pong (designed: Plan 0014 `PingPongField`)

Read one texture, write the next, fade each step. Ideal iGPU fit — no per-cell branching cost.

| Technique | What it is | Audio fit | Cost |
|-----------|-----------|-----------|------|
| **Reaction-diffusion (Gray-Scott)** | two chemicals diffuse + react → coral/spots/stripes/mitosis; knobs `F` (feed), `K` (kill), `Da/Db` | High — tiny F/K space onto bands | Moderate — **Plan 0014** |
| **Lenia (continuous CA)** | states in [0,1]; `A += dt·G(K∗A)` — radial kernel convolution + unimodal growth `G(mu,sigma)` | High — mu/sigma/dt are small expressive knobs | Moderate; cost dominated by kernel radius `R` |
| **Conway / discrete CA** | Game of Life + generalised grid rules | Med — beat → reseed, amplitude → update rate | Cheap |
| **Walker trails** | any walker family (below) rendered into a fading feedback texture | see walkers | Trivial once the fade pass exists |

## Idiom D — full-screen fragment (have it: `fragment_field.rs`)

One quad, all colour in the pixel shader. New looks are new shaders on the existing pattern +
named params; no new audio wiring.

| Technique | What it is | Audio fit | Cost |
|-----------|-----------|-----------|------|
| **Chladni / cymatics plates** | zero set of `cos(n·pi·x/L)cos(m·pi·y/L) − cos(m·pi·x/L)cos(n·pi·y/L)`; integer mode numbers `n,m` | High — `n,m` from beat / dominant frequency; **thematically perfect for a music viz** | **Cheapest of all** — no state, pure per-pixel eval |
| **Domain-warped noise fields** | iterated Perlin/simplex fold + cosine palette | High — warp/zoom/hue params | Cheap (this is essentially what `fragment_field` already does) |
| **Raymarched SDF scenes** | CSG tree of signed-distance primitives, sphere-traced in the fragment shader | Med-High — primitive params, camera | **Budget-sensitive** — cost scales with march steps; needs per-scene budgeting on iGPU |
| **MilkDrop-style per-pixel fields** | per-pixel + per-frame equations + pixel shader (the projectM design) | Very high (the field's reference design) | Moderate |

---

## Walker family (rides idiom A or C)

All are one compute/update step + one fade pass — four presets for roughly one scene's cost.
Ranked by fit for a beat-reactive, iGPU-friendly engine:

1. **Correlated / persistent walk** — heading = previous + small random turn. Exposes the two most
   musical knobs: **step size** (→ amplitude) and **turn bias** (→ spectral centroid). Trivial. **Sweet spot.**
2. **Noise-guided walk** — heading read from a Perlin field; "combed flow" look. Trivial.
3. **Flow-field walkers** — shared field; audio moves *all* walkers coherently (beat → radial impulse). Moderate.
4. **Lévy flight** — heavy-tailed step length; gate the long jumps on beat → punchy synchronized leaps. Trivial.
5. Classic / drunkard's walk — calm, diffusive; no natural turn-bias knob. Trivial.
6. Self-avoiding walk — history-dependent, fights audio forcing, awkward on GPU. Mod-heavy.

Structural / ambient (better as **capped CPU generators** for occasional non-reactive scenes):
**differential growth**, **space colonization** (branching venation/trees).

## The Nature of Code → scene mapping

Daniel Shiffman's free book (natureofcode.com) maps almost 1:1 onto reusable scenes. Highest-leverage
chapters, priority order:

1. **Ch. 3 Oscillation** — sine/polar/harmonic motion; essentially *free* (vertex math), extends the
   Maurer-rose / parametric-curve family directly.
2. **Ch. 4 Particle Systems** — the archetypal visualizer scene (idiom B).
3. **Ch. 0 Randomness** — the walkers above.
4. **Ch. 7 Cellular Automata** — ping-pong texture, ideal iGPU fit (idiom C).
5. **Ch. 5 Autonomous Agents** — flocking/boids (needs a spatial-hash grid).

Deprioritise / offline: Ch. 6 Physics (heavy solver); **Ch. 9 Evolutionary** (great *offline* — evolve
preset parameters toward an aesthetic fitness; ties into the deferred preset-author skill); Ch. 10/11
Neural (audio-analysis helper at most, not a renderer).

## What real music visualizers actually use (verified)

- **MilkDrop / projectM** — the field's reference design and most-deployed visualizer (Winamp, Kodi):
  per-pixel + per-frame equations + pixel shaders, driven by FFT bass/mid/treble + beat over PCM — the
  exact features this project already extracts.
- **Shadertoy-style** — a single full-screen quad, all visuals in the fragment shader, audio fed as a
  spectrum texture or named uniforms.
- **VJ tools (TouchDesigner / Hydra)** — the same idioms wrapped in node graphs.

---

## Suggested pick order (payoff ÷ engine work)

1. **Superformula / harmonograph / epicycloid** — extend the *current* line renderer (Plan 0010),
   near-zero new infra. Immediate wins.
2. **Idiom B GPU particles → strange attractors first** — cheapest high-impact new idiom; unlocks
   curl-noise + fractal flames later. **This is [Plan 0016](plans/0016-gpu-compute-particle-scenes.md).**
3. **Idiom D → Chladni, then SDF** — new shaders on the existing fragment field; Chladni is nearly free
   and thematically on-point for a music visualizer.
4. **Idiom C → Gray-Scott (Plan 0014), then Lenia.**

Walkers ride idiom A or C, so they slot in cheaply alongside.

## Honest caveats (from the research)

- **Coverage gap:** no verified source survived for L-systems, discrete Game-of-Life CA, standalone
  noise/domain-warp fields, harmonograph/epicycloid, or metaballs/marching-squares. That reflects the
  verified-claim set, **not** infeasibility — all are well-known and cheap.
- **Performance figures are partly extrapolated:** SDF raymarching, large-radius Lenia convolutions, and
  dense reaction-diffusion scale with per-pixel work — budget per scene on integrated GPUs.
- **Chaos amplifies cross-vendor floating-point divergence** (attractors especially), so golden-image
  regression should assert structural metrics or use the software-adapter baseline + tolerance, not
  pixel-exact compares.

## Sources (verified pass)

glChAoS.P (GPU attractor particles) · rreusser "Strange Attractors on the GPU" · Draves *The Fractal
Flame Algorithm* (flam3.com) · Codrops "Reaction-Diffusion Compute Shader in WebGPU" · Quilez
raymarching SDFs (iquilezles.org) · Bourke Chladni figures (paulbourke.net) · Wikipedia Lenia · Gielis
superformula (GPU-SuperFormula3D) · projectM · Audio-Shader-Studio (audio→uniform recipe) ·
natureofcode.com (Ch. 0/3/4/5/7 confirmed against live TOC) · Generative Hut random walkers · Sighack
Perlin flow fields.
