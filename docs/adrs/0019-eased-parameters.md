# ADR-0019 — Eased (smoothed) parameters: render-layer one-pole filtering on injected `dt`, expression layer stays pure

> **Status:** accepted
> **Date:** 2026-07-23
> **Related plan(s):** [0018-engine-wide-visual-enrichment](../plans/0018-engine-wide-visual-enrichment.md); extends [0002](0002-layered-preset-architecture.md) layer 2; depends on Plan 0014's injected `dt`

## Context

Named parameters (ADR-0002 layer 2) are **pure, stateless expressions evaluated every frame**:
`render/mod.rs::draw_frame` calls `reset_params()` then `set_param(name, expr.eval(vars))` for
each binding, and the scene uses the value *immediately*. The expression evaluator is pure and
**allocation-free**, guarded by a zero-alloc test (Plan 0003 / ADR-0003).

The live smoke of the line scenes surfaced the cost of "immediate": band-driven params track the
raw, noisy per-frame band value, and beat-driven params snap on the beat — the user's "changes in
shapes are very rigid and fast". The fix is easing (a low-pass / slew on the value over time).

Two things make *where* to ease a real decision:

- **Smoothing needs state and a time step.** A one-pole filter needs the previous value and a
  `dt`. Putting state into the expression layer (a stateful `smooth()` / `slew()` builtin) would
  break the evaluator's purity, its zero-alloc guarantee, and its "pure function of the input
  window" determinism (NFR §6). The render layer, between `eval` and `set_param`, keeps the
  expression layer pure.
- **The `dt` must be real.** On today's fixed `1/60` `SCENE_DT` the filter is frame-rate-coupled,
  which contradicts NFR §6 and the user's stated "same on every device" goal. Plan 0014 injects a
  real `dt` at the render seam and retires `SCENE_DT`; smoothing built on it is frame-rate-
  independent from day one. So this is **sequenced after Plan 0014**.

## Decision

We will ease parameters in the **render layer**, between evaluation and application. Each
evaluated value passes through an optional **one-pole low-pass** (exponential smoothing) with a
per-parameter time constant `tau`, using Plan 0014's injected `dt`:

> `alpha = 1 - exp(-dt / tau)` ; `smoothed += alpha * (evaluated - smoothed)` ; then
> `set_param(name, smoothed)`.

Per-`(preset, param)` previous-value state lives in the renderer and is **reset on preset switch**
(a switch snaps to the new preset's first evaluated value — no cross-preset bleed) and on any
scene rebuild for capture (so a headless capture stays a pure function of its inputs, Plan 0013).
The **expression layer stays pure, stateless, and allocation-free** — unchanged.

`tau` comes from an optional **`[smoothing]` preset table** (`param = seconds`); a param not
listed uses a small default time constant (the plan pins the exact default; `tau = 0` means "no
smoothing", i.e. today's behaviour). Determinism holds: given the same `dt` sequence and the same
inputs from a known reset state, the smoothed output is a pure function.

Discrete/structural params (`visible_depth`, `variant`) are smoothed as continuous values, but a
scene still floors the result to pick a cached state — so this eases their *pre-floor* motion, not
the structural snap itself. A true **crossfade between cached states** (morph between L-system
depths / tiling variants) is a separate, deferred concern (a Plan 0010 followup), not this ADR.

## Consequences

### Positive
- **Engine-wide with zero scene changes** — the filter sits at the shared `set_param` seam, so
  every scene family (fragment, swarm, lines, and future ones) is eased for free.
- The **expression layer stays pure** — no regression to the zero-alloc / determinism guarantees.
- **Per-param control** via `[smoothing]`: a punchy `zoom` pump and a slow `hue` drift can carry
  different time constants in the same preset.
- **Frame-rate-independent** (built on Plan 0014's injected `dt`), matching NFR §6 and the
  "identical on every device" goal.

### Negative (the price we pay)
- **Introduces per-param state in the renderer** — it must be reset on preset switch *and* on the
  scene rebuild the Plan 0013 capture path does, or a capture stops being a pure function of its
  inputs. A missed reset is a determinism bug, not just a visual one.
- **A new preset-schema surface** (`[smoothing]`) — one more optional table to document and
  validate at load.
- **Smoothing adds latency** — that is the point, but an over-large `tau` makes a scene feel
  sluggish; the per-param `tau` and a conservative default keep it in the author's hands.
- **Discrete params still snap** — easing their pre-floor value does not morph structure;
  crossfade is future work.
- **Depends on Plan 0014** (injected `dt`).

### Neutral
- **C ABI untouched** — smoothing is internal to the render layer; `[smoothing]` is a preset
  concept both frontends inherit through the shared preset library.
- A preset with no `[smoothing]` table behaves as today, modulo the (small, optionally zero)
  default `tau`.

## Alternatives considered

### Alternative A — A stateful `smooth()` / `slew()` expression builtin (ease in layer 1)
Add a smoothing function to the expression language so a preset writes `smooth(bass, 0.1)`.
Rejected: the evaluator is deliberately pure and allocation-free (a guarded invariant, ADR-0003),
and per-call state would break purity, the zero-alloc test, and the "pure function of the input
window" determinism. Smoothing is a render-time concern with a `dt`, not an expression concern.

### Alternative B — Per-scene smoothing (each scene eases its own params)
Let each scene low-pass the values it receives. Rejected: every scene family reimplements the same
filter and its reset logic; the fix belongs once at the shared `set_param` seam.

### Alternative C — Smooth now on the fixed `1/60` `SCENE_DT`
Ship easing immediately on today's clock. Rejected (the user's call at the interview): it is
frame-rate-coupled — it contradicts NFR §6 and the "same on every device" goal, and would need
re-tuning when Plan 0014 lands the injected `dt`. Sequence after 0014 and do it once, correctly.

### Alternative D — One global smoothing constant (no per-param `tau`)
A single time constant for all params. Rejected: different params want different responsiveness (a
beat-driven zoom pump vs a slow hue drift); per-param `tau` via `[smoothing]` is cheap and the
whole point.

## Notes
- Pairs with [ADR-0018](0018-engine-wide-scene-compositing.md) (the GPU composite) under Plan 0018.
- The exact default `tau` (including whether the default is `0` = off) is a plan-level tuning
  decision, validated on the iGPU test box against the "not rigid, not sluggish" feel.
