# ADR-0026 — Full composite coverage: background + view transform for the fullscreen/accumulating scenes (reaction-diffusion, attractor)

> **Status:** proposed
> **Date:** 2026-07-24
> **Related plan(s):** [0025-full-composite-coverage](../plans/0025-full-composite-coverage.md); extends [0018-engine-wide-scene-compositing](0018-engine-wide-scene-compositing.md)

## Context

ADR-0018 made the engine own a fixed-order composite — `background pre-pass -> active scene (with a
shared ViewTransform) -> trails -> kaleidoscope -> present` — and exposed each stage as ADR-0002
named params so audio can drive it. The intent was **engine-wide** coverage. In practice two of the
five levers reach only three of the (now) five colored scenes:

- **View transform (`zoom`/`pan_x`/`pan_y`)** is consumed by `fragment_field`, `swarm`, and the line
  scenes. It is delivered *through named params* — each scene takes `zoom`/`pan_*` via `set_param`
  and applies the transform in its own sample/particle space. `reaction_diffusion` and `attractor`
  never wired it up, so those params are silently ignored on them (there is no `deny_unknown_fields`;
  the `set_param` `_ => {}` arm swallows them).
- **Background (`bg_hue`/`bg_bright`/`bg_vignette`)** draws first, but `reaction_diffusion` and
  `attractor` both **present opaque** to the engine target (`reaction_diffusion.rs` returns
  `vec4(out_col, 1.0)` with `BlendState::REPLACE`; the attractor's final present outputs `vec4(c, 1.0)`
  opaque). A fullscreen opaque present overwrites the backdrop the `bg_*` pass drew, so no atmosphere
  can ever show behind these scenes.

This surfaced from the `preset-author` lane while authoring the "Chthonic Coral Oracle" coral preset
(design-backlog entry 0001): the coral look is mostly dark voids between contours, and there is no way
to fill those voids with the tintable gradient — the two most natural composite levers are no-ops on
the exact scene where they would matter most.

The **geometry mirror (`mirror_*`)** is deliberately line-only (segment replication before the cap,
ADR-0018) and is **not** in scope — a fullscreen field has no segments, and the screen-space
kaleidoscope already supplies that symmetry. `trails` and the kaleidoscope already work on both scenes
(scene-agnostic post-passes). So the gap is exactly *background* and *view transform* on the two
fullscreen/accumulating scenes.

The decision is whether these scenes should join the ADR-0018 composite the same way the others do
(compositing over the backdrop), or stay self-contained — and it could reasonably go either way,
because "composite over the backdrop" changes how existing `reaction_diffusion`/`attractor` presets
look (their voids stop being unconditionally black).

## Decision

We will **extend the ADR-0018 composite to cover `reaction_diffusion` and `attractor`**, using the
same delivery mechanisms the covered scenes already use — no new seam:

> **Background.** Both scenes' **final present to the engine target** switches from an opaque
> `REPLACE` to an **alpha-blend `OVER` with `LoadOp::Load`**, emitting an alpha derived from the
> scene's own structure (reaction-diffusion: the contour/field `structure` term; attractor: the
> accumulated luminance). Where the scene has nothing (V≈0 voids, empty attractor space) the pixel is
> transparent and the `bg_*` backdrop shows through. Each scene's *internal* offscreen/accumulation
> clears are untouched — only the surface present changes.
>
> **View transform.** Both scenes accept `zoom`/`pan_x`/`pan_y` via `set_param` (exactly as
> `fragment_field` does) and apply them in their own space: reaction-diffusion transforms the present
> pass's sample UVs; the attractor folds them into its world projection. No change to the `Scene`
> trait — the transform continues to arrive through the existing named-param path.

Default background brightness is low (a dark backdrop), so a shipped preset that binds no `bg_*`
renders essentially unchanged; the alpha present over a black backdrop equals the old opaque black.
Presets that *do* bind `bg_*` gain the atmosphere. This is the "full audit, all scenes" scope: every
colored scene gets every applicable composite lever.

## Consequences

### Positive
- The two most valuable composite levers become real on the two scenes where authors most want them —
  coral voids (and attractor negative space) fill with the tintable `bg_*` gradient.
- **Uniform mental model:** every colored scene now honors `zoom`/`pan_*` and `bg_*`; a preset author
  no longer has to memorize which scene silently drops which param.
- **No new seam.** View transform stays a named-param concern; background stays the existing pre-pass.
  The `Scene` trait and C ABI are untouched — same shape as ADR-0018.

### Negative (the price we pay)
- **Existing `reaction_diffusion`/`attractor` presets can change appearance** once a backdrop is
  non-black: their voids are no longer unconditionally black. Pre-1.0 we accept re-authoring over a
  compat shim; the shipped coral trio should be spot-checked and re-blessed. Golden captures for these
  two scenes will shift and must be re-blessed (a deliberate, reviewed re-bless, not a silent diff).
- **Alpha-blended present is slightly more work** than an opaque `REPLACE` (a blend per pixel). Trivial
  at these fill rates, but it must still hold the NFR §1 iGPU 60 fps floor — verify on the test box.
- **Two present shaders now compute an alpha channel** they didn't before; a wrong alpha term reads as
  a washed or punched-through scene. Caught by the re-bless review, but it is real surface area.

### Neutral
- **C ABI untouched** — core-internal, driven by named params; the foobar frontend inherits it via
  presets.
- `mirror_*` remains line-only by design; this ADR does not touch it.

## Alternatives considered

### Alternative A — Self-contained backdrop inside each scene's shader
Keep both scenes opaque; have each read `bg_*` and paint its own gradient into its own voids. Rejected:
it duplicates the background logic that already exists as a dedicated pre-pass (ADR-0018), giving two
backdrop implementations that can drift, and it still would not deliver `zoom`/`pan_*`. Compositing
over the existing backdrop reuses one mechanism for all scenes.

### Alternative B — Leave RD/attractor opaque; document them as "no background/zoom"
Accept the gap as a designed limitation (backgrounds are for the line/particle scenes). Rejected: the
scene where the backdrop matters most (coral's dark voids) is exactly the one excluded, and "which
params work on which scene" is precisely the silent-footgun the layered preset model tries to avoid.
The naming of this rejected option is what makes the decision ADR-worthy.

### Alternative C — Add a `Scene::set_view(ViewTransform)` hook instead of routing through params
Give scenes an explicit view setter. Rejected: `zoom`/`pan_*` already flow through `set_param` for the
covered scenes, and widening the `Scene` trait for something the named-param path already carries is a
seam change (ISP) for no gain. Match the existing delivery.

## Notes
- Builds directly on [ADR-0018](0018-engine-wide-scene-compositing.md); does not reopen its fixed-order
  vs render-graph decision.
- Interacts with [Plan 0020 / ADR-0021](0021-shared-palette-system.md) (shared palette) — both touch
  the RD/attractor present shaders. If the palette work lands first, the alpha-present change rides on
  top of the LUT-colored output; sequence is a plan-ordering note, not a conflict.
- NFR §1 (iGPU 60 fps) governs the added blend cost; NFR §6 (determinism) is unaffected — the present
  stays a pure function of the field/accumulator.
