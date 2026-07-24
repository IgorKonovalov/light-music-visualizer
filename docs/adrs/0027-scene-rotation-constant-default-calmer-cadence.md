# ADR-0027 — Scene rotation: hold one scene by default, calmer cadence, softened drop bias

> **Status:** accepted
> **Date:** 2026-07-24
> **Related plan(s):** [0026-calmer-scene-rotation](../plans/0026-calmer-scene-rotation.md); revises the defaults set by [Plan 0009](../plans/done/0009-live-performance-features.md) (`standalone/src/director.rs`, `config.rs`)

## Context

Plan 0009 built the standalone "live show" director (`standalone/src/director.rs`): an auto-rotate
state machine that cycles presets on a MilkDrop-style dwell timer, biased to rotate sooner on an
energy **drop** and nudged by a track-change **novelty** signal, with manual `Space` override and a
`toggle_auto` hotkey. Its config defaults (`config.rs::Rotate`) are:

- `auto = true` — the app auto-rotates out of the box.
- `min_dwell_secs = 8`, `max_dwell_secs = 40`.
- `track_change = true` (novelty nudge on).

Those defaults encode a deliberate Plan 0009 stance: the primary use case is an unattended stage show,
so rotate automatically and keep it lively. In practice the user reports the opposite need: scenes
**change too fast**, and the desired default is a **constant scene** you sit on until you choose
otherwise. The "too fast" is not just `min_dwell`: the drop-bias fires a rotation at the *min* dwell
(8 s) on any 35%-below-baseline energy dip, and dynamic music dips often — so the effective cadence in
real audio is roughly "every 8 s," not "every 40 s." The novelty nudge pulls the cap in further.

So this is not a bug in the director — it works as designed. It is a **defaults + tuning** decision
that reverses Plan 0009's product stance, which is exactly what warrants a recorded decision: the
"unattended lively show" default is a nameable alternative we are choosing against for the common case.
Scope is standalone-only (`director.rs` + `config.rs`); the core engine and C ABI are untouched.

## Decision

We will make **holding one scene the default**, and make rotation — when a user opts in — **calm and
predictable rather than frantic**:

> - **`auto` defaults to `false`.** First run (no config) holds a single scene indefinitely. The user
>   opts into rotation via the existing `toggle_auto` hotkey or `config.toml`. Manual `Space`
>   next-scene keeps working whether or not auto is on (unchanged).
> - **Longer dwell when auto is on:** `min_dwell_secs` 8 -> **20**, `max_dwell_secs` 40 -> **90**
>   (defaults only; config still overrides).
> - **Soften the drop bias, don't remove it.** An energy drop may still rotate early, but only *well
>   past* the longer min dwell — the drop trigger is gated by a larger floor so it can no longer flip
>   scenes every few seconds. The novelty nudge stays but rides the same longer dwell.

The director's mechanism (drop bias, novelty nudge, manual override, deterministic injected-`dt`
clock) is unchanged in shape; we retune its constants/gate and flip one default. Because every
`Rotate` field is `#[serde(default)]`, an existing user's `config.toml` that pins these values keeps
its behavior; only a fresh install gets the new defaults.

## Consequences

### Positive
- The out-of-the-box experience matches the common expectation — pick a look, it **stays** — while the
  hands-off show is one hotkey away.
- When rotation is on, the cadence is **calm and mostly predictable** (timer-led), with drops still
  able to land a change on a real section boundary rather than on every dip.
- Tiny, well-contained change: two default flips plus one gate constant, all in the standalone shell;
  fully unit-testable in `director.rs`'s existing deterministic test style.

### Negative (the price we pay)
- **Reverses Plan 0009's "lively unattended show" default.** A user who wanted auto-rotate now has to
  enable it once (hotkey or config). Mitigated by the hotkey and by persisting the choice.
- **New default numbers (20/90) are a judgment call** — the real calibration is on-device (same
  posture as Plan 0009's on-rig soak). They may need one more pass after live use.
- Keeping the drop bias (rather than going timer-only) preserves a little unpredictability; that is the
  explicit tradeoff chosen over strict metronomic rotation, so "why did it change there?" can still
  have a musical answer.

### Neutral
- **Core, C ABI, and the foobar frontend are untouched** — the director lives in the standalone shell.
- Config schema shape is unchanged; only default *values* move, so no migration.

## Alternatives considered

### Alternative A — Keep `auto = true`, just lengthen the dwell
Stay a hands-off show by default but rotate far less often. Rejected as the default: the user's stated
need is a *constant* scene out of the box, not merely a slower carousel. (This remains available to any
user via one config line — it is a supported configuration, just not the default.)

### Alternative B — Timer-only rotation (drop the energy-drop and novelty triggers)
Rotate purely on the dwell timer for full predictability. Rejected per the interview: keep the drop
bias but soften it, so a scene change can still align with a genuine section drop instead of landing
mid-phrase — the musicality is worth a little unpredictability.

### Alternative C — Remove auto-rotate entirely, manual-only
Delete the director; `Space` is the only way to change scenes. Rejected: the unattended show is a real
use case (Plan 0009's whole point) and the machinery already exists and is tested — defaulting it off
preserves the capability at zero ongoing cost.

## Notes
- Standalone-only; no core/DSP/ABI impact. The director's determinism (pure function of injected `dt` +
  `AnalysisFrame`, NFR §6) is preserved — retuning constants doesn't touch that.
- Exact softened-drop gate and the 20/90 defaults are set in [Plan 0026](../plans/0026-calmer-scene-rotation.md)
  and are on-device-tunable, not frozen here.
