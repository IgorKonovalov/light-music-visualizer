# ADR-0007 — Line-geometry generators: a cached-build built-in category + instanced-quad line rendering

> **Status:** accepted
> **Date:** 2026-07-21
> **Related plan(s):** [0010-line-geometry-scenes](../plans/0010-line-geometry-scenes.md)

## Context

The engine's built-in system vocabulary (ADR-0002 layer 2) currently renders two ways:
a full-screen fragment field and a ~10k-point additive swarm. Both are **per-frame-cheap** —
a shader over the whole frame, or a fixed particle count re-simulated each frame — and both
carry only continuous named parameters (`warp`, `hue`, `force`, ...). We want to add a family
of **line-art generative systems** inspired by three existing sketches (Maurer rose, L-systems,
Islamic star patterns via the Hankin method). None of the current vocabulary can express them:
there is no line/curve rendering primitive, and two of the three are not per-frame-cheap.

Three forces make this a real decision rather than "just add another scene":

- **wgpu has no usable thick line.** `PrimitiveTopology::LineList` draws 1px lines whose width
  is fixed and inconsistent across DX12/Metal — unacceptable for a glowing, projector-scale
  visual. Line thickness must be produced by us, not the backend.
- **Two of the three families are expensive to *build*, cheap to *animate*.** A Maurer rose is
  a pure parametric curve — a few thousand `sin`/`cos`, safe to regenerate every frame. An
  L-system expands a grammar exponentially (depth 6 of a branching rule is 10^5+ segments);
  the Hankin construction intersects tiling lines. Recomputing either per frame would blow the
  60 fps @ 1080p iGPU floor (NFR §1) and violate the sacred-hot-path rule. Their *structure*
  changes rarely (on preset load, or a beat); their *appearance* (rotation, hue, draw-on
  progress) changes every frame.
- **The structural inputs are not expressions.** A grammar's rules or a tiling's contact angle
  cannot be a pure `f32`-valued expression string (ADR-0002 layer 1). They are static
  declarative data that a Rust generator consumes — a shape the current preset schema
  (`system` + `params: map<string,expr>`) has no slot for.

ADR-0002 anticipated this: it fixed the built-in vocabulary deliberately and said extending it
with a new *category* is an ADR-gated event, not a casual edit. This is that event.

## Decision

We will add a **line-geometry category** to the layer-2 vocabulary, built on one shared GPU
line renderer and split into two build models by cost.

1. **One line renderer, thick lines as instanced quads.** A crate-internal `LineRenderer`
   helper expands each segment into a camera-facing quad in the vertex shader (width, glow, and
   additive blend as uniforms) — the swarm scene's instanced-quad pipeline with segments in
   place of points. Scenes hand it a segment buffer; it owns the pipeline and the draw. We do
   **not** use native wgpu line primitives.

2. **Two build models, chosen by cost, sharing that renderer:**
   - **Parametric** (`parametric_curve`) — a pure `t -> (x, y)` curve **sampled every frame**
     into a capped segment buffer (allocation-free, reusing a preallocated buffer). Maurer rose
     is a preset of this. Continuous audio sweep (`n`, `d`, `scale`, `hue`, ...) is the natural
     motion.
   - **Generator** (`lsystem`, `star_pattern`) — expensive structure **built off the hot path**
     (on preset load, and optionally advanced on a beat), cached as segment buffers. Per frame
     the scene only *picks* a precomputed state and applies a cheap transform/color. Structure
     is never expanded or intersected inside a render frame.

3. **Structural inputs are static TOML data; continuous inputs stay expressions.** The preset
   schema gains an optional per-system config table (`[curve]` / `[generator]`) holding the
   declarative structure (curve family; axiom, rules, angle, depth; tiling type, contact angle).
   Everything that animates stays an expression binding under `[params]`. A generator receives
   its config through one new **optional** `Scene` hook (`configure`, defaulting to no-op like
   `set_param`) invoked at preset load — off the hot path — so the vocabulary stays thin and
   non-generators stub nothing.

4. **The geometry source stays engine-internal.** Generators (grammar+turtle, Hankin) are
   crate-private Rust producing segment buffers — pure, deterministic, unit-tested — not a new
   public plugin seam. The extension surface remains the two seams ADR-0002/ADR-0001 fixed
   (the C ABI and the thin `Scene` trait); this widens `Scene` by exactly one optional method.

For the "both continuous base + beat accents" requirement (the plan's audio model): continuous
motion is per-frame transform/hue/draw-progress; beat accents advance a *precomputed* index
(grow an L-system one iteration, swap a tiling variant) — never a live regeneration.

## Consequences

### Positive
- A whole class of line-art systems (roses, curves, fractal growth, tilings) becomes available,
  and most future curves are **pure data presets** over the parametric system — no Rust per
  pattern (open/closed: adding a rose variant edits no engine code).
- The perf floor is protected by construction: the only per-frame work is bounded sampling or a
  cached-buffer upload + uniform write. Exponential/quadratic build cost lives at preset load.
- Thick, glowing, anti-aliased lines that look identical on DX12 and Metal, reusing a pipeline
  shape already proven by the swarm scene.
- Generators are pure functions of their config + seed — determinism (NFR §6) holds and is
  directly unit-testable (axiom+rule+depth -> exact segment count).

### Negative (the price we pay)
- **A new preset-schema surface.** The `[curve]`/`[generator]` config table is a new
  compatibility contract alongside the expression grammar — presets authored today must keep
  loading. It needs the same care as the C ABI and expression language.
- **`Scene` gains a method.** `configure` widens the trait we promised to keep minimal. It is
  optional (default no-op) and off the hot path, but it is a real widening the reviewer must
  watch doesn't grow further.
- **Precompute is bounded.** Beat-driven structural change can only move between states cheap
  enough to precompute at load. Genuinely unbounded live structural morphing (e.g. a
  continuously varying grammar angle rebuilding every frame) is out of reach until we add a
  background-thread regeneration path — deliberately deferred, not smuggled in.
- **Real build cost.** The grammar/turtle and Hankin generators are non-trivial geometry code
  before the first on-screen payoff for those two families (the rose pays off immediately).

### Neutral
- Line scenes live under `core/src/render/scenes/lines/`, inside the existing recursive
  hot-path hygiene scan (Plan 0002) — the per-frame files inherit the panic pragma; the
  build-time generator files are colocated but not per-frame (pragma placement is a review item).

## Alternatives considered

### Native wgpu line primitives (`LineList`/`LineStrip`)
Zero geometry expansion — hand the GPU vertices and draw lines. Rejected: width is locked near
1px and varies by backend, so a projector-scale glowing visual is impossible and the look would
differ between the standalone (DX12) and any Metal target. Instanced quads cost a little more
memory for full control.

### Regenerate structure every frame
Simplest generator model — no caching, no lifecycle hook; the scene just rebuilds from config
each frame. Rejected outright: exponential L-system expansion and quadratic Hankin intersection
in the render loop violate the sacred hot path and cannot hold 60 fps. This is the whole reason
the parametric/generator split exists.

### Background-thread regeneration as the primary model
Regenerate on a worker thread and swap buffers when ready, allowing unbounded live structural
params. More capable than precompute, but adds threading, double-buffering, and swap-latency
complexity for a v1 that the precompute model already satisfies (continuous sweep + beat-indexed
states). Kept as the documented **escape hatch** for a later plan, not the v1 mechanism.

### Structural inputs expressed in the expression language
Keep the schema unchanged by encoding grammars/tilings as expression strings. Rejected: the pure
`f32`-valued expression grammar (ADR-0002) cannot represent a production rule set or a tiling
topology, and stretching it to try would balloon the very surface ADR-0002 kept small.

### A public geometry-source plugin seam
Expose the generator as a third extension point so new generators plug in without touching the
engine. Rejected for v1: it contradicts ADR-0002's "keep the seams to two" and ADR-0001's
minimal-surface stance, and we have no external author for it yet. Generators stay crate-private
Rust; revisit only if outside authorship becomes a goal (its own ADR).

## Notes
- The three source sketches are JavaScript on canvas2D/SVG; none of the code is reused. What
  ports is the math — polar rose sampling, grammar expansion + turtle interpretation, and the
  Hankin contact-angle construction — reimplemented in Rust against the shared line renderer.
