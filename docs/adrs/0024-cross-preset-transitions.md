# ADR-0024 — Cross-preset transitions: a two-input blend stage over the engine composite, adaptive dual-live/freeze, engine-default policy

> **Status:** proposed
> **Date:** 2026-07-23
> **Related plan(s):** [0023-cross-preset-transitions](../plans/0023-cross-preset-transitions.md); builds on [0018](0018-engine-wide-scene-compositing.md) (the engine composite's offscreen target + present pass) and [0012](0012-stateful-feedback-render-system.md)/[0013](0013-c-abi-v4-render-dt.md) (Plan 0014's `PingPongField` + injected `dt`); realizes the "cross-preset blending" follow-up deferred by [Plan 0003](../plans/done/0003-generative-scenes-and-presets.md)

## Context

Switching presets today is an **instant cut**: `cycle_preset`/`select_preset` bump `Roster.active`
(an index) and the next frame draws the newly-active scene, clearing straight to the swapchain
(`render/mod.rs::draw_frame`). MilkDrop's signature is the opposite — presets *dissolve* into one
another over roughly a second, which reads as one continuous show rather than a slideshow. The
user asked for that.

A blend fundamentally needs **two scene outputs available in the same frame** and a stage that
mixes them by a factor `t`. Neither exists today: there is exactly one live scene, and it renders
directly to the surface with no offscreen capture. Plan 0018 (ADR-0018, approved, sequenced ahead)
introduces the missing backbone — an engine-owned offscreen target, a present pass, and scenes that
**stop clearing** (`Clear` -> `Load`). A transition is the natural second consumer of that backbone:
render two composited frames into two targets, blend, present.

Three forces shape the decision:

- **The 60 fps @ 1080p iGPU floor (NFR §1) is a hard constraint.** Running two full composite chains
  (each: background -> scene+view -> trails -> kaleidoscope) every frame doubles GPU cost for the
  transition window. The heavy stateful families — the compute-particle attractor (Plan 0016, 100k
  particles) and Gray-Scott reaction-diffusion — can blow the budget if blended live against each
  other.
- **A preset switch can resolve to the *same* scene object.** The active scene is derived
  `system_slot(preset.system)`; two presets on the same `SystemKind` (e.g. two fragment fields)
  select the **one** prebuilt scene instance. You cannot render one mutable, stateful object into
  two targets as "two different presets" in a single frame.
- **The blend must sample both inputs, not alpha-composite one over the other.** The line and
  particle families draw with **additive** blending; compositing an outgoing frame with per-pixel
  alpha over an incoming one produces wrong colors and cannot express non-crossfade transitions
  (wipe, luma-dissolve, burn). A real blend stage that samples two textures is required.

## Decision

We add an engine-internal **transition controller** and a **two-input blend stage** appended to the
Plan 0018 composite. The mechanism is asymmetric on purpose:

> At transition start, **snapshot the outgoing composited frame** into a texture. Each transition
> frame, render the **incoming** preset live through the composite into target A; produce the
> **outgoing** input either from the snapshot (freeze) or, when eligible, by re-rendering the
> outgoing scene live into target B (dual-live). **Blend(outgoing, A, t, kind) -> present**, with
> `t` advancing over the configured duration on injected `dt`.

The snapshot is both the **freeze path** and the **safety net**. **Dual-live is an opportunistic
upgrade**, taken only when *both* hold: (1) the outgoing and incoming presets resolve to **different
scene objects** (so two live renders are even possible), and (2) the smoothed frame time (from the
Plan 0011 `FrameStats`/`Diag` clock) is under budget. If either fails — same scene slot, or the
budget is blown mid-transition — the controller falls back to the frozen snapshot for the remainder.
This is the **adaptive** behavior the user chose, and it makes the same-slot case correct for free.

The blend operates on **fully-composited per-preset frames** (each preset's own background, view,
trails, and kaleidoscope), so a transition dissolves the complete look, not a raw scene layer.

**Transition policy (which kind, how long) is engine-configured in code** — a default duration and a
`TransitionKind` (or a rotation over the library) chosen at the switch site. The library is a small
fixed set: **crossfade, additive/burn, luma-dissolve, and wipe/slide**, each a blend-shader variant
selected by an enum. Preset-declared transitions (a `[transition]` TOML table) are a deliberate
**follow-up**, not this decision.

**The C ABI is untouched.** The transition fires from inside the existing `cycle`/`select` paths and
runs entirely within the render loop; the foobar frontend gets dissolves for free through the same
`lmv_render_dt` call. No new `extern "C"` surface, no `Scene`-trait widening — the incoming and
outgoing scenes are driven through the render call exactly as today.

## Consequences

### Positive
- Preset switches become **continuous dissolves** on both frontends with no C ABI or `Scene`-trait
  change — the shared-core design pays off.
- **Reuses Plan 0018's offscreen target + present pass**; the transition is one more fixed stage
  (blend) after the composite, not a parallel pipeline. `t` on injected `dt` makes a transition
  frame-rate-independent and, given a fixed start snapshot + seeded scenes, reproducible in the
  Plan 0013 capture harness.
- **Adaptive freeze keeps the iGPU floor safe.** The expensive dual-composite only runs when the
  budget allows and the scenes differ; everything else pays one composite + a static-texture blend.
- The same-slot problem dissolves out of the mechanism (same object -> always freeze) rather than
  needing a special case.

### Negative (the price we pay)
- **A second full-frame target + a blend pass per transition frame**, and in dual-live, a **second
  composite chain** (two backgrounds, two trail fields, two kaleidoscopes). Real cost, bounded to the
  transition window and governed by the adaptive fallback — but it must be measured against NFR §1 on
  the test box.
- **Freeze mode shows the outgoing preset as a still image** for the dissolve. It is not the
  live-warping-both-presets MilkDrop ideal on every transition — only when dual-live is eligible.
  Accepted as the floor-safe default.
- **A stateful incoming scene may hitch at transition start** — the lazy first-render GPU build
  (documented at Plan 0014 close) now lands at the dissolve's opening frame, the worst moment.
  Mitigation (pre-warm on `begin_transition`) is a plan risk, not settled here.
- **Re-entrancy surface.** A switch arriving mid-transition, a hot-reload (`set_presets`), or a
  browse-overlay explicit select must be defined (snap-finish vs. restart). More states to test.

### Neutral
- **C ABI untouched**; policy lives in engine code, so tuning duration/kind is a code edit, not an
  ABI or schema change.
- The blend stage is an engine-internal pipeline stage, not a new plugin seam — the two extension
  seams (C ABI, thin `Scene` trait) keep their shape.
- Hard-couples this initiative to **Plan 0018 landing first** (its offscreen target + present + the
  `Clear`->`Load` scene migration are preconditions), the same way 0018 couples to Plan 0014.

## Alternatives considered

### Alternative A — Single target, alpha-composite the outgoing over the incoming
Draw the outgoing frame at `1-t` alpha over the incoming, one target, no blend shader. Rejected: the
additive line/particle families do not alpha-composite to correct colors, and a single alpha lerp
cannot express wipe/luma-dissolve/burn. The user chose a **small transition library**, which requires
a stage that samples both inputs.

### Alternative B — Always dual-live (both presets simulate through the whole blend)
The true MilkDrop feel — both presets warp and react while dissolving. Rejected as the *default*: two
full composites every transition frame risks the 60 fps iGPU floor on the heavy stateful families, and
it is simply impossible when both presets share one scene object. Kept as the **opportunistic** path
inside the adaptive decision.

### Alternative C — Always freeze (snapshot outgoing, never re-render it live)
The cheap, always-safe path. Rejected as the *ceiling*: on light, different-slot transitions we can
afford both live and the user chose **adaptive**, so we take dual-live when it is free. Freeze remains
the fallback and the same-slot behavior.

### Alternative D — Model the transition as a `TransitionScene` wrapping two child scenes
A composite `Scene` owning outgoing + incoming and the blend, slotted into the roster so the render
loop stays single-active. Rejected: it fights the prebuilt-singleton scene registry (the line scenes
share one `Rc<RefCell<LineRenderer>>`; a wrapper would double-borrow), it pushes engine-lifecycle and
blend knowledge into the `Scene` trait that ADR-0002 keeps deliberately thin, and it still cannot make
one stateful object render two states. The controller lives in the render loop, above the trait.

### Alternative E — Preset-declared transitions now (`[transition]` TOML table)
Let each incoming preset own its dissolve (kind, duration), MilkDrop-authentic and preset-author
facing. Rejected **for now**, not on merit: it adds preset schema (an ADR-0002 supplement) and author
surface on top of an already large plan (library + adaptive governor). Engine-default policy ships the
visible feature; preset-declared transitions are a clean follow-up on this backbone.

## Notes
- Sequenced after Plan 0018 (offscreen target + present + `Clear`->`Load` scenes are preconditions);
  transitively after Plan 0014 (its `PingPongField` + injected `dt`).
- NFR §1 (iGPU 60 fps) governs the adaptive dual-live/freeze budget; NFR §6 (determinism) holds because
  `t` accumulates injected `dt` from a fixed start snapshot over seeded scenes.
- The adaptive governor reuses the Plan 0011 `FrameStats`/`Diag` frame-time clock — no new timing source.
- Pairs with, and is sequenced behind, [ADR-0018](0018-engine-wide-scene-compositing.md); realizes the
  cross-preset blending deferred in [Plan 0003](../plans/done/0003-generative-scenes-and-presets.md).
