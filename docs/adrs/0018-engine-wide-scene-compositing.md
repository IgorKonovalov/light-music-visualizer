# ADR-0018 — Engine-wide scene compositing: shared view transform, background pre-pass, feedback trails, and screen-space post-effects (fixed order, not a render graph)

> **Status:** proposed
> **Date:** 2026-07-23
> **Related plan(s):** [0018-engine-wide-visual-enrichment](../plans/0018-engine-wide-visual-enrichment.md); builds on [0012](0012-stateful-feedback-render-system.md)/[0013](0013-c-abi-v4-render-dt.md) (Plan 0014's offscreen field + injected `dt`); extends [0002](0002-layered-preset-architecture.md) layer 2

## Context

Every scene today owns its own frame: `Scene::render` clears the surface view and draws in one
pass, and the only compositing the engine does is the existing scene -> text -> overlay `Load`
passes in `render/mod.rs::draw_frame`. Three capabilities the live smoke of the line scenes
surfaced as missing all sit *outside* a single scene's draw:

- **A shared view (zoom/pan).** Line scenes expose a per-shape `scale`, but nothing zooms or
  pans the *view*. Zoom/pan is a camera concept every scene family should share, not a param
  each reinvents.
- **A backdrop behind the strokes.** Each line scene hard-clears to near-black, so there is no
  atmosphere behind the geometry. A background is a pass that must run *before* the scene and be
  owned by something other than the scene (which no longer clears).
- **Effects over the finished frame.** Motion trails and a screen-space kaleidoscope both need
  the *composited* frame as input — a fade-and-accumulate feedback of it, or a mirror-fold of
  it. Neither can be expressed inside a single scene's draw; they are post-processes.

Plan 0014 (approved, sequenced first) introduces exactly the machinery these need:
`render::feedback::PingPongField` (two offscreen `Rgba16Float` textures + a present pass that
composites offscreen -> surface) and an **injected real `dt`** at the render seam
(`Renderer::render(&frame, dt)`, retiring `SCENE_DT`). Trails *are* a `PingPongField`; a
screen-space kaleidoscope needs the scene rendered into a sampleable offscreen texture — the
same present machinery. So this work is naturally **sequenced after 0014 and reuses its
primitives** rather than duplicating them.

The real decision is *structure*: a general, data-driven render graph, or a minimal fixed-order
composite? With four effects (background pre, view transform, trails feedback, kaleidoscope
post) plus the existing text/overlay, a fixed order is enough; a general graph is more
abstraction than four effects justify (the same YAGNI call the project made against a
background-regen worker in ADR-0007).

## Decision

We will make the **engine own an ordered composite**, reusing Plan 0014's offscreen target and
present pass, in this fixed order:

> **background pre-pass** (gradient/vignette clear) -> **active scene**, drawn with a shared
> **`ViewTransform`** (zoom / pan / rotate about centre) into the engine-owned offscreen target
> -> **feedback trails** (fade + accumulate via `PingPongField`) -> **screen-space kaleidoscope**
> (fold into N mirrored wedges) -> **present to the surface** -> existing text/overlay passes.

Scenes **stop clearing their own view** (`Clear` -> `Load`) and draw into the engine target; the
background pass owns the clear. Every effect is individually skippable — a preset that binds none
pays only a passthrough present. The order is fixed in the render loop; it is **not** a general
render graph.

The `ViewTransform` is a shared uniform each scene family applies in its own space (line scenes
multiply endpoint positions in the `LineRenderer` vertex shader; fragment scenes transform their
sample coordinates; the swarm multiplies particle positions). It is an engine concept exposed as
ADR-0002 named params (`zoom`, `pan_x`, `pan_y`), so audio can drive it.

The **geometry mirror stays separate and line-only** (the "both mirrors" product decision): line
scenes replicate their segment set under N-fold rotation + reflection *before* the segment cap,
producing a true geometric fractal; the **screen-space kaleidoscope** is the general pixel-fold
post-pass over the finished frame. Both ship; they are different looks (a fold of pixels is not a
fractal built from the geometry).

The eased-parameter layer that also feeds this composite is a separate decision — [ADR-0019](0019-eased-parameters.md).

## Consequences

### Positive
- Zoom, background, trails, and kaleidoscope become **engine-wide and audio-bindable** through
  the existing named-param seam — no per-scene reinvention.
- **Reuses Plan 0014's `PingPongField` + present pass** — no duplicate offscreen/feedback infra,
  and the injected-`dt` clock makes the time-based effects frame-rate-independent.
- A fixed pass order is simple to reason about, diff, and golden-test; scenes get *simpler*
  (they no longer own the clear).
- Most future look-effects (a bloom pass, a color grade) slot into the same fixed order as one
  more optional stage.

### Negative (the price we pay)
- **Every scene must stop clearing** — a coordinated one-time change across all scene families. A
  scene that still clears would wipe the background; the Plan 0013 sanity/golden suite catches a
  regression, but the migration touches every scene once.
- **Always rendering through an offscreen target + present** adds one full-frame present per
  frame even when no effect is active (a passthrough cost). This must hold the 60 fps @ 1080p
  iGPU floor (NFR §1) — measured on the test box; if it regresses, the passthrough can bypass the
  offscreen hop when no effect is bound.
- **The fixed order can't express arbitrary effect graphs** (e.g. kaleidoscope-before-trails).
  Accepted; revisit with an ADR if a third post-effect needs reordering.
- **Hard-couples this initiative to Plan 0014 landing first** (offscreen/present + injected `dt`).

### Neutral
- **C ABI untouched.** The composite is core-internal and driven by named params; no new
  `extern "C"` surface. The foobar frontend gets it for free via presets.
- The composite is an engine-internal pipeline, not a new public plugin seam — the two extension
  seams (C ABI, thin `Scene` trait) are unchanged in shape (scenes gain no new *required* method;
  the `ViewTransform` arrives through the existing render call).

## Alternatives considered

### Alternative A — A general, data-driven render graph
An ordered pass/effect list scenes and effects register into. Rejected: four fixed effects plus
text/overlay do not justify the abstraction, and a graph is materially more machinery to build,
test, and keep correct. Premature (YAGNI) — revisit when effect count or a real reordering need
demands it.

### Alternative B — Per-scene ownership (each scene does its own zoom/background/trails)
Keep the single-pass model and add the four effects inside every scene. Rejected: each scene
family reimplements the same four effects, and a *screen-space* post (kaleidoscope, trails of the
composited frame) cannot even be expressed per-scene — it needs the finished frame. Duplication
plus an impossible case.

### Alternative C — Screen-space kaleidoscope only (drop the geometry mirror)
One pixel-fold post-pass, no line-geometry replication. Rejected by the product decision: a fold
of a line drawing's *pixels* is not the same as a true geometric fractal built from the
*segments* (different symmetry centre behaviour, no true line continuity across wedges). Both
looks are wanted, so both ship.

### Alternative D — Build our own offscreen + present instead of reusing Plan 0014's
Stand up an independent offscreen target/present just for these effects. Rejected: it duplicates
the exact `PingPongField` + present machinery Plan 0014 introduces. Sequence after 0014 and reuse
it; if 0014's field proves too rigid, that is a 0014 revision, not a parallel copy here.

## Notes
- Sequenced after Plan 0014 (its offscreen/present + injected `dt` are preconditions).
- NFR §1 (iGPU 60 fps) governs the passthrough/offscreen cost; NFR §6 (determinism) governs the
  time-based effects — trails accumulate a fixed `dt`, so a capture is reproducible when the
  field is reset on scene rebuild (Plan 0013 capture path).
- Pairs with [ADR-0019](0019-eased-parameters.md) (eased parameters) under Plan 0018.
