# Plans index

The one-minute "what's in flight" view. Read this first each session instead of
re-deriving state from `git log`. Completed plans move to `done/`.

**Next free number: 0004**

## Active roster

| Plan | Title                                   | Status | Summary |
|------|-----------------------------------------|--------|---------|
| [0001](0001-core-and-standalone-mvp.md) | Core + standalone MVP, then foobar parity | in-progress | Workspace → CI → Win loopback → DSP → wgpu spectrum → scenes → C ABI → foobar SDK (human) → plugin → mac capture → mac validation (human). Phases 0–5 landed (ring, capture, DSP, render, scenes); Phase 6 (C ABI) next. Bars come from [docs/nfr.md](../nfr.md). |
| [0002](0002-rust-enforcement-tooling.md) | Rust enforcement tooling | approved | Automatic gates for the best-practice rules: rustfmt + workspace lints → clippy determinism bans → hot-path panic-denial + exact-pin/pragma guard tests → cargo-deny → nextest → Miri. Strict but rational. |
| [0003](0003-generative-scenes-and-presets.md) | Generative scenes + data-driven presets | draft | Shadertoy-style fragment-field scene + ~10k-particle CPU swarm, driven by TOML+expression presets (ADR-0002 layers 1-2). DSP enriched with bass/mid/treb + deterministic tempo/BPM. Defers Rhai, blending, compute-scale. Drafts roadmap item 1. |

**Execution note:** Plan 0001 has advanced faster than 0002 was drafted — Phases 2–5 (the
lock-free ring, WASAPI capture, DSP, render, scenes) already landed. So 0002 now serves two ends:
it still wants to run **before 0001's Phase 6 (C ABI)** to arm the gates ahead of the FFI `unsafe`,
and it **retroactively hardens the already-written hot-path code** — expect its first run to
require adding the `#![deny(...)]` pragma to the existing `dsp`/`audio`/`render` modules and to
surface any latent `unwrap`/indexing in the ring and DSP. That retroactive shakedown is a feature,
not rework.

## Roadmap (agreed 2026-07-21, revised same day for the live-show use case; numbers assigned when drafted)

Execution order after Plan 0001, per the NFR interviews ([docs/nfr.md](../nfr.md)):

1. **Preset / scripting engine** — layered presets per
   [ADR-0002](../adrs/0002-layered-preset-architecture.md): TOML data + expression language
   driving built-in systems (feedback/warp, boids, walkers/growth, 3D scene), with an
   optional budgeted Rhai script for staged per-track arcs (NFR §10). Replaces "scenes are
   Rust code" — Plan 0001's Scene trait becomes the rendering vocabulary presets drive, so
   keep it thin. **Started as [Plan 0003](0003-generative-scenes-and-presets.md)** (layers 1-2:
   fragment-field + swarm systems, data + expression presets); Rhai (layer 3), blending, and
   compute-scale particles remain follow-ups tracked in 0003.
2. **Live performance features** — line-in/audio-interface capture, scene triggers
   (auto-rotate + hotkey/MIDI + experimental track-change detection), fullscreen on a
   chosen display/projector, 4-hour soak stability (NFR §10).
3. **Adaptive quality** — quality tiers + frame-time governor for the 60 fps iGPU floor
   (NFR §1). Validated on the older iGPU test PC.
4. **Remaining v1 UX** — always-on-top / mini mode, settings persistence (NFR §11;
   fullscreen/multi-monitor land earlier with live features).
5. **Packaging & release** — GitHub release zip: unsigned standalone exe +
   `.fb2k-component` (NFR §8).

Later, unordered: better tempo tracking, preset sharing/library, signed installer.

## Conventions

- **Numbering:** sequential, zero-padded 4 digits. Take the next free number above, then
  bump it here in the same session.
- **Phases:** ordered, each one commit, each tagged `**Owner skill:**` with one value from the
  vocabulary `dev` (all code) or `human` (a task only the user can do). The `dev` skill reads
  this tag at the start of each phase; a missing tag is a Mode 4 review blocker. An optional
  `**Area:**` note (`core` / `standalone` / `plugin`) orients the reader but is not the tag.
- **Skills:** `architect` designs and owns `docs/`; `dev` implements all code. `architect`
  writes and closes plans; `dev` flips `draft → in-progress` at "go" and nothing else in the file.
- **Lifecycle:** `draft` → `approved` (user/architect validated it; ready for `dev`) →
  `in-progress` → `done` (then `git mv` to `done/` and drop from this roster). Review
  happens at plan end, in a fresh `/architect` session — not by the session that wrote
  the code.
