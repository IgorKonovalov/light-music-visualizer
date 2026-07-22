# ADR-0010 — Accept the DX12/wgpu driver-stack memory floor; retarget the runtime-memory NFR

> **Status:** accepted
> **Date:** 2026-07-22
> **Related plan(s):** [0011](../plans/done/0011-diagnostics-and-memory-trim.md) (built the diagnostics harness and measured the floor)
> **Amends:** [docs/nfr.md](../nfr.md) §12

## Context

NFR §12 (added at the Plan 0001 close) set a runtime-memory target of "standalone steady-state
working set **well under ~100 MB**" and named the **primary lever** as compiling wgpu with only the
per-OS backend (DX12 on Windows / Metal on macOS), dropping the Vulkan/GL paths as "dead weight."

Plan 0011 built the diagnostics harness to measure that lever and then landed it (Phase 6, wgpu gated
to the per-OS backend + explicit swapchain depth). Phase 7 (human smoke, 2026-07-22, Windows AMD iGPU,
**release** build) measured the result and disproved the premise:

- **Working set ~300 MB, private commit 343 MB** — *above* the ~200 MB baseline, far above the <100 MB
  target. The trim did not reduce footprint.
- The trim **did take effect**: verified DX12-only at runtime (no `vulkan-1.dll` / `opengl32.dll`
  mapped). So the lever worked as designed; it simply targets the wrong thing.
- Root cause, measured not guessed: mapped DLL **code** is only 135 MB (and shared across processes);
  the cost is **private heap commit**, dominated by the **DX12 driver stack** — largest module
  `amdxc64.dll` (AMD DX12 driver / shader compiler) 44.8 MB, plus `d3dcompiler_47`, `D3D12Core`,
  `d3d11`, `d2d1`, `dcomp`. That stack loads and allocates the same regardless of which backends wgpu
  compiled in. Dropping Vulkan/GL Rust code cannot touch it.

NFR §12 already suspected "the footprint is almost entirely the GPU stack" — the diagnosis was right,
but it mistook the *GPU stack* for wgpu's compiled backend code when it is the **driver's** DLLs and
private heap. The footprint even grew vs the 200 MB baseline, most plausibly because Plans 0003/0010/0011
added render pipelines/shaders (each driver-compiled pipeline grows that heap).

## Decision

We accept the **GPU driver stack as a fixed, dominant, vendor-dependent cost** and retarget NFR §12
away from an absolute low-water mark it cannot reach, to requirements the Plan 0011 diagnostics harness
can actually enforce:

1. **Drop the "well under ~100 MB" absolute.** On a DX12/wgpu app the driver floor alone exceeds it
   before we draw anything reducible.
2. **No session growth (leak guard).** Private commit / working set must stay flat over a session — no
   monotonic growth over the §10 ≥4-hour soak. This is the requirement that actually protects a live
   show, and the harness (`diagnostics.log`) is the instrument.
3. **State the incremental cost of what we add.** The actionable lever is **our** additions —
   render-pipeline/shader/resource count — not backend code. A new built-in system states its
   working-set delta on the reference box (harness-measured), so growth is a conscious, recorded choice.
4. **A soft ceiling for regression-catching, not an absolute.** ~350 MB working set on the reference
   AMD iGPU box with the current built-in system set — a single-machine, vendor-dependent number whose
   role is to catch a regression, not to certify an absolute footprint.

The exact bare-wgpu/DX12 driver floor (our overhead vs the fixed stack) was **not isolated** in this
pass; a small dev spike (a scene-less wgpu window measured on the same box) can refine points 3–4 with a
hard floor number later, but does not change this decision.

## Consequences

### Positive
- The RAM NFR is now **honest and harness-enforceable** — the leak guard + per-system delta are things
  Plan 0011's instrument measures directly, and the leak guard ties straight into the Plan 0009 soak.
- Memory effort points at the real lever (pipeline/shader/resource count), not a retired one.
- "Lightweight is a feature" still binds **what we add** — it stops binding the driver floor we don't own.

### Negative (the price)
- We publicly concede a GPU visualizer's working set is driver-dominated and vendor-variable — the
  clean "<100 MB" story is gone. A future need for a genuinely tiny footprint would mean a different
  rendering strategy (an ADR-0001-superseding decision), not a tweak.
- The soft ceiling is one machine's number; on a different GPU/driver the floor differs, so the ceiling
  is a regression tripwire, not a portable guarantee.

### Neutral
- The Phase 6 backend-trim **stays** — it is a legitimate binary-size win (NFR §4) and removes dead
  code paths; it is simply not a *memory* lever. No revert.

## Alternatives considered

### Alternative A — Keep the <100 MB target, pursue heavier levers to hit it
Chase the number with drastically fewer pipelines, aggressive resource sharing, or a headless/low-tier
mode. Rejected: the measured driver floor (~300 MB WS on the AMD iGPU) exceeds 100 MB before we draw a
single reducible thing, so we would be contorting the visual system toward a number the driver stack
makes unreachable. Reducing pipeline count is still worthwhile (point 3), but as incremental savings on
top of a fixed floor, not in service of an impossible absolute.

### Alternative B — Drop the runtime-memory NFR entirely
Delete §12; declare memory out of scope for a GPU app. Rejected: we still need the **leak guard** (a
4-hour live show cannot grow unbounded) and we still want each added system's cost to be a recorded
choice. "Lightweight" still applies to what we control.

### Alternative C — Switch GPU abstraction or go GPU-less to shrink the floor
Rejected: the driver stack is inherent to any hardware-accelerated renderer, and abandoning wgpu
contradicts ADR-0001. This is a floor of the chosen architecture, not a wgpu-specific tax.

## Notes

Supersedes nothing. Amends NFR §12 (the file is updated in lockstep with this ADR). The measurement
method is recorded for repeatability: PowerShell `Get-Process lmv` → `WorkingSet64` vs
`PrivateMemorySize64`, `.Modules` sorted by mapped size, and a check for which backend loader DLLs are
mapped — the private-vs-working-set split is what proved the cost is driver heap, not our code.
