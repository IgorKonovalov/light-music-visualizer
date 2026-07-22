# On-device validation — low-end Windows iGPU smoke

> **Status:** standing / hardware-gated — **does not block plan closes.**
> **Owner:** human (the user; only runnable on the target hardware).
> **Created:** 2026-07-22 (extracted from Plan 0012 Phase 3).

This is a **checklist, not a phased plan.** It exists so that on-device checks the user can
only run "much later" — when the low-end / older Windows iGPU test box is in hand — never sit
in the plan roster reading as stalled work and never gate a plan from closing. Development and
plan-close momentum stay unblocked; these items get ticked whenever the hardware is available.

Keep it lightweight: add an item when a plan produces an iGPU-hardware check it can't run
itself, tick items when the user runs them, and route any failure back to `dev`/`architect` as
its own follow-up. When every open item is ticked and nothing new is pending, this file can be
deleted.

## Why these live here (not in a plan)

The §9 test matrix names an "older Windows PC (iGPU)" the dev box is not. Everything measured so
far is one machine, one GPU vendor (AMD). Two questions can only be answered on that other
hardware, and the user won't have access to it until later:

1. Does the **NFR §1 perf floor** (≥ 60 fps @ 1080p at the shipped single fixed tier) hold on the
   weakest box?
2. Is the **~350 MB working-set soft ceiling** (NFR §12) AMD-specific, or does a second GPU vendor
   (Intel iGPU) land somewhere different?

Neither blocks shipping — they confirm portability of a floor and a ceiling already accepted on
the dev box (ADR-0010). So they wait here.

## Reference baseline (dev box — what the low-end box is compared against)

Release build, AMD iGPU dev box, post-cull (2-scene) standalone, 1080p, steady state
(measured 2026-07-22, `diagnostics.log`, ~5.5 min run):

| Metric | Dev-box value |
|--------|---------------|
| fps | ~165 (0 dropped frames over 51k) |
| frame_ms avg / p99 | ~6.06 / ~6.9 ms |
| working set (`rss_bytes`) | ~303 MB (run-to-run noisy; private commit ~338 MB is the stable figure) |
| gpu_bytes | ~16.6 MB |

The low-end box need not match these — it need only clear the **≥ 60 fps** floor and report *its*
footprint so the vendor spread is on record.

## Checklist

- [ ] **Low-end / older Windows iGPU box (§9), 1080p.** Run the current release standalone, let
      it reach steady state, capture `diagnostics.log`. Report **(a)** fps holds ≥ 60 @ 1080p, and
      **(b)** steady-state working set + private commit. _(This is Plan 0012 Phase 3, extracted; it
      also satisfies the identical Plan 0003 Phase 3 iGPU-60-fps carry-forward — same measurement.)_
- [ ] **Second GPU vendor — Intel iGPU, if a box is available**, 1080p. Same capture. The point is
      the footprint spread vs the AMD dev box — confirms whether the ~350 MB ceiling is AMD-specific.

## How to run

From the repo root on the target box:

```
cargo build -p standalone --release --bin lmv
./target/release/lmv.exe
```

Play any audio (loopback capture feeds the visuals). Then, in the window:

- **`Space`** — cycle presets (step through all 10; each should render and react).
- **`F3`** — toggle the diagnostics overlay (frame-time sparkline + GPU bar + fps/p99 readout).

The 1 Hz log lands at:

```
%APPDATA%\light-music-visualizer\diagnostics.log
```

Columns: `unix_ms  fps  frame_ms_avg  frame_ms_p99  frames_total  frames_dropped  gpu_bytes  rss_bytes`.
`rss_bytes` is the working set. For private commit too, run the throwaway floor spike or read
`PrivateMemorySize64` via `Get-Process lmv` (the ADR-0010 method).

## Pass criteria & escalation

- **Pass:** fps ≥ 60 @ 1080p (NFR §1 floor holds) and a recorded working-set / private-commit
  figure for the box.
- **Fps below 60** → a §1 floor regression on the weakest box → route to `dev`/`architect` as a
  new follow-up (this is the trigger the adaptive-quality-tier plan waits on).
- **A wildly different vendor footprint** (e.g. Intel far above or below the AMD ~350 MB ceiling)
  → route to `architect` to widen the NFR §12 soft ceiling from one-vendor to a measured spread.

## Provenance

Extracted from Plan 0012 Phase 3 at that plan's close (2026-07-22) so Plan 0012 could close on its
two completed `dev` phases (scene cull + driver-floor spike) without waiting on hardware. See
`docs/plans/done/0012-memory-floor-measure-and-scene-cull.md` and
[ADR-0010](adrs/0010-accept-gpu-driver-memory-floor.md).
