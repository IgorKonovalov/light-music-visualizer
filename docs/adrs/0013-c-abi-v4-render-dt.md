# ADR-0013 — C ABI v4: add lmv_render_dt (injected real dt); bump to v4

> **Status:** proposed
> **Date:** 2026-07-22
> **Related plan(s):** 0014-reaction-diffusion-feedback-scene
> **Related ADRs:** 0003 (C ABI v1), 0006 (v2), 0008 (v3), 0012 (feedback system)

## Context

The feedback simulation system (ADR-0012) is driven by a fixed-timestep accumulator
that needs the real elapsed time between frames. That `dt` is **injected** at the render
entry point — not read from a wall clock inside `core` — so the core stays clock-free
(the single gated clock read Plan 0011 quarantined in `Diag` remains the only one) and
headless capture (Plan 0013) stays reproducible by feeding a fixed `dt`. On the native
Rust path this is `Renderer::render(&frame, dt)`.

The foobar plugin renders live through the C ABI, whose `lmv_render(handle)` (ADR-0003,
frozen v1; extended v2/v3) takes no timing argument. Plugin parity is valued — the
simulation should be frame-rate-independent on that path too, not only in the standalone.
The C++ shim compiles against `core/include/lmv_core.h` separately from the Rust crate,
so changing that surface is an ADR-worthy event (CLAUDE.md).

The project has a consistent ABI-evolution pattern: every prior extension **added a
function and bumped the version** (v2 added `lmv_load_presets`, ADR-0006; v3 added the
diagnostics pair, ADR-0008) rather than changing an existing signature. This decision
follows that pattern.

## Decision

We will add one function — `int32_t lmv_render_dt(LmvHandle *handle, float dt_seconds)` —
and bump `LMV_ABI_VERSION` from 3 to 4. `lmv_render(handle)` stays, now defined as a thin
wrapper that calls the `dt` path with the legacy fixed `1.0/60.0` step, so existing hosts
keep linking and behaving exactly as before. The C++ shim measures elapsed wall-clock
`dt` on its render thread and calls `lmv_render_dt`, giving the plugin the same
frame-rate-independent simulation as the standalone. `dt` is passed in and never read by
the core, preserving the clock-free-core rule and capture reproducibility.

## Consequences

### Positive
- The plugin reaches full frame-rate independence — parity with the standalone on the
  simulation path.
- Additive and non-breaking: hosts still calling `lmv_render` keep working; the surface
  grows by one function, matching the v2/v3 precedent.
- `core` stays clock-free — the wall-clock read lives in the host shim, where per-frame
  timing belongs.

### Negative
- The ABI surface grows to nine functions at version 4; `lmv_core.h`, `core/src/ffi.rs`,
  and the C++ shim must move in lockstep (the hand-maintained-header cost — no cbindgen,
  per ADR-0003).
- Two render entry points to keep coherent: `lmv_render` must remain exactly the
  `1/60` wrapper over `lmv_render_dt`, or the two paths silently diverge.

### Neutral
- `LMV_ABI_VERSION` 3 → 4; hosts gate on it via `lmv_abi_version()` as before. No struct
  crosses the ABI for this call — a single `float` in, `int32_t` status out.

## Alternatives considered

### Alternative A — Change lmv_render's signature to take dt
A breaking change to a frozen contract: every host must update in lockstep and old
components fail to link. Rejected — the project's ABI discipline is additive-plus-version-
bump, not signature churn.

### Alternative B — Core reads a monotonic clock for the plugin path
No ABI change, but it puts a wall-clock read back inside `core`, violating the clock-free
rule Plan 0011 preserved with its single gated read, and it makes the plugin simulation
non-reproducible. Rejected.

### Alternative C — Leave the plugin on a fixed 1/60
No ABI change, but the plugin simulation then drifts with its render cadence — the exact
divergence ADR-0012 eliminates on the standalone — forfeiting the valued plugin parity.
Rejected; the additive function is cheap enough to just do now.
