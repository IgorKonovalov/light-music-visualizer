# Plans index

The one-minute "what's in flight" view. Read this first each session instead of
re-deriving state from `git log`. Completed plans move to `done/`.

**Next free number: 0012**

## Active roster

| Plan | Title                                   | Status | Summary |
|------|-----------------------------------------|--------|---------|
| [0008](0008-preset-browse-overlay.md) | In-app preset browse overlay (standalone) | approved | Give the standalone a keyboard-driven browse overlay over the running visual: a key opens a scrollable list of preset names, arrows move a highlight, typing narrows it (type-to-filter), Enter jumps straight to that preset, Esc closes. Needs the codebase's first text rendering — **glyphon** behind a core `text` cargo feature ([ADR-0009](../adrs/0009-glyphon-text-rendering.md)), via a reusable `render::text` seam Plan 0009's HUD later shares. Adds `Renderer::preset_names`/`select_preset`; overlay logic is a pure unit-tested `OverlayState` in the shell. Standalone-only (plugin stays cycle-only), keyboard-only, C ABI untouched. The interview's chosen selection UX, split out of Plan 0007. |
| [0009](0009-live-performance-features.md) | Live performance features (standalone) | approved | Drive a live DJ show onto a projector from the standalone: borderless-fullscreen on a chosen display, line-in / audio-interface capture (alongside loopback), self-rotating scenes (energy/drop-biased) with a manual hotkey override, experimental track-change detection (core DSP nudge), and a ≥4-hour instrumented soak. Operator choices persist in a per-user `config.toml`. Standalone-only via the native Rust API + one deterministic DSP field in core; C ABI frozen, no ADR. MIDI deferred. Roadmap item 2 (NFR §10). |
| [0010](0010-line-geometry-scenes.md) | Line-geometry scenes: parametric curves, L-systems, star patterns | approved | Add a line-art category to the built-in system vocabulary, ported from the user's Maurer rose / L-systems / Islamic star pattern sketches. One shared `LineRenderer` (segments -> instanced quads, thick + glowing) under two build models: a cheap **parametric** system sampled per frame (the rose) and an expensive **generator** system built at preset load and cached (L-systems, star patterns). Continuous audio drives transform/hue/draw-on every frame; beat accents advance precomputed structural states. New `[curve]`/`[generator]` TOML config table + one optional `Scene::configure` hook ([ADR-0007](../adrs/0007-line-geometry-generators.md)); extends ADR-0002 layer 2. Core-only, C ABI frozen. Preset files ride Plan 0007's seeding. |

## Recently closed

- [0011 — Diagnostics harness + quick-win memory/perf trim](done/0011-diagnostics-and-memory-trim.md) —
  **done 2026-07-22**, passed Mode 4 review (no blockers, no majors; two nits). Seven phase commits
  (`7ad00df`, `166043f`, `5a9f67b`, `1ace817`, `82c7134`, `d266c08`) plus two post-review fixes
  (`10a4796`, `894a2fc`). Built the runtime diagnostics brain in `core`: a pure `FrameStats`
  accumulator (fps / frame-ms / p99 from a fixed 240-sample ring, unit-tested, no clock) wrapped by a
  `Diag` holding the **single gated `Instant::now()` read** — the only wall-clock read in `core`,
  quarantined behind `collecting` so NFR §6 determinism (fixed `SCENE_DT`) holds. A `render/overlay.rs`
  final pass paints a frame-time sparkline + GPU bar + a dependency-free 5x7 bitmap-digit readout as
  instanced quads (off by default, skipped when off). Standalone: F3 toggle, dependency-free per-OS RSS,
  1 Hz rotating `diagnostics.log` on the render thread. Foobar plugin reaches the same overlay + metrics
  over new **C ABI v3** (`lmv_set_debug` + `lmv_get_metrics` + size-guarded `LmvMetrics`,
  [ADR-0008](../adrs/0008-c-abi-v3-diagnostics.md), now **accepted**) — the v3 FFI test rides in with a
  `static_assert(sizeof == 56)` layout guard. Phase 6 landed the NFR §12 levers: wgpu gated to the per-OS
  backend only (DX12/Metal, default-features off, dropping the Vulkan/GL dead weight) and an explicit
  2-frame swapchain latency. `diag/` joined the hot-path panic-pragma guard + `hygiene.rs` scan set.
  **⚠ Phase 7 outcome (human smoke, 2026-07-22, Windows AMD iGPU):** fps unchanged (~165 @ 1080p — no
  §1 regression) and overlay/title parity verified, **but the §12 footprint win failed** — release
  `lmv.exe` measured ~300 MB WS / 343 MB private, *above* the 200 MB baseline. Measured root cause: the
  trim took effect (DX12-only verified, no Vulkan/GL mapped) but footprint is dominated by the DX12
  driver-stack private heap (`amdxc64.dll` 44.8 MB + `d3dcompiler`/`D3D12Core`), which the backend-trim
  can't touch. **Backend-trim retired as the memory lever; §12's <100 MB target likely unreachable on
  DX12/wgpu.** → **Follow-up (new work, does not reopen 0011):** measure the bare wgpu driver floor,
  then revise NFR §12 or profile pipeline/shader count as the real lever. Still-standing on-device
  checks: live-foobar overlay/log (like Plan 0004) and macOS RSS (`rss.rs`, pending a Mac — Plan 0001).
  **Nits (non-blocking):** (a)
  `LmvMetrics.draw_calls` counts render passes, not GPU draw calls — name slightly wider than the value;
  (b) `foo_lmv.cpp` adds a third hardcoded app-dir literal (the Plan 0007 shared-path minor, not new).

- [0007 — Curated preset library: robust loading + seed-on-first-run + C ABI v2](done/0007-curated-preset-library.md) —
  **done 2026-07-22**, passed Mode 4 review (no blockers, no majors). Four phase commits
  (`448b54b`, `ac5e7d0`, `cf8fb5b`, `ed67807`): `core::preset::seed_dir` (write-if-absent) +
  a hand-rolled per-OS data-root resolver in the standalone seed `%APPDATA%\light-music-visualizer\presets`
  on first run, then load + hot-reload it; the foobar shim resolves the **same** dir and calls
  the new `lmv_load_presets` after every `ensure_handle`, gated on an `lmv_abi_version()`
  handshake, so both frontends share one on-disk library. The C ABI grew by exactly one
  function and bumped to **v2** ([ADR-0006](../adrs/0006-c-abi-v2-preset-loading.md), now
  **accepted**) — the first automated FFI test rides in with it (create -> load_presets on a
  temp dir -> assert count + seeded + null-path error), closing the 0001/0002 zero-FFI-coverage
  gap. Curated set expanded 4 -> 10 (calm/warp/bright fragment + drift/dense/storm swarm
  variants). A `pending_presets` stash on `RenderState`, drained by `lmv_attach_window`, handles
  a load-before-attach call order (matching ADR-0006's "install" intent). Selection stays cycle +
  title-bar; the in-app browse overlay is **Plan 0008** (drafting next). Delivers roadmap item 1's
  preset-library thread + part of item 5's install-readiness.
  **⚠ Carry-forward (human):** (a) Phase 3 live foobar smoke — builds x64 Release against v2;
  seeding + Next-scene cycling in a running foobar2000 is an on-device check (Plan 0001 Phase 8
  posture). (b) Phase 4 visual quality — "visibly distinct/reactive" across the 10 presets is an
  on-box judgment. **Minor (non-blocking):** the shared preset-path convention is a string literal
  in both frontends (`standalone/src/main.rs`, `foo_lmv.cpp`) with no single source of truth — a
  rename silently un-shares them; a cross-referencing comment is the follow-up.
- [0004 — foo_lmv as an embeddable Default UI panel](done/0004-foobar-ui-element-panel.md) —
  **done 2026-07-21**, passed Mode 4 review (no blockers, no majors). All four phases landed in
  `plugin-foobar/foo_lmv.cpp` (commits `ef9193f`, `be3f90c`, `49ed225`, `855ccba`): the file-scope
  globals became one claimable `VizSession` (single `LmvHandle` + stream + pump + render timer); a
  Default UI `ui_element` panel and the View pop-out both host the core through one HWND, sharing
  the session so only one wgpu surface exists; ownership arbitration (400 ms poll) hands the session
  to a still-open host when the owner frees, with a GDI placeholder for non-owners; "Next scene" via
  right-click + Space; and a visibility/playback-driven cadence (full while playing+visible, ~6-7 fps
  idle, timer off when hidden). **Plugin-only, no ADR** — diff touches only `foo_lmv.cpp`, the C ABI
  is unchanged (`LMV_ABI_VERSION` still 1, only the pre-existing surface called), and the
  single-`lmv_create` invariant is owner-gated on both create paths. Relates to roadmap item 4 (UX).
  **⚠ Carry-forward:** all four done-whens are runtime checks in a live foobar2000 v2 — the code
  implements each; behavioral confirmation is pending an on-device run.
- [0005 — Extract the lock-free ring into a wgpu-free crate for Miri](done/0005-miri-ring-extraction.md) —
  **done 2026-07-21**, passed Mode 4 review (no blockers, no majors). Implements Plan 0002's
  deferred Phase 5. Phase 1 (`de0fe24`) pulled the SPSC ring — `RingShared`, `SampleProducer`,
  `SampleConsumer`, `spsc()`, and the four SPSC unit tests — out of `core/src/audio.rs` into a
  new zero-dependency `lmv-ring` crate, re-exported unchanged from `core::audio` (public API and
  the C ABI intact). The ring types carry a bare `channels: u16` instead of the core-owned
  `AudioFormat` (which stays at the `intake()` boundary with its validation), driving one
  documented `capture_win.rs` call-site edit — the plan's own Risks-section fallback.
  `hygiene.rs` guards extended to cover `lmv-ring` in both the exact-pin and hot-path-pragma
  checks. Phase 2 (`6af7865`) added the `miri` CI job (`cargo +nightly miri test -p lmv-ring`) —
  fast because no wgpu graph compiles; the probe (Release→Relaxed → data-race UB) confirmed the
  gate bites. No ADR (internal refactor; the rejected feature-gate-wgpu alternative is recorded
  in the plan). **⚠ Carry-forward:** the Miri job's green-in-CI is a runtime check pending the
  push (needs the `workflow` OAuth scope on the git credential). **Minor (non-blocking):**
  `spsc()` is now crate-public in a `publish=false` crate — a slightly wider surface than the
  former module-private constructor; the `channels`-validated-by-caller contract is documented.

- [0003 — Generative scenes + data-driven presets](done/0003-generative-scenes-and-presets.md) —
  **done 2026-07-21**, passed Mode 4 review (no blockers). Phases 0-5 landed (commits
  `ae2c035..df16c48`): scenes relocated under `render/` + brought under the panic-pragma guard
  (closing the 0002 review gap), a fragment-field system and a ~10k CPU particle swarm, DSP
  enriched with bass/mid/treb bands + a deterministic hop-clock tempo/BPM, a pure
  allocation-free expression evaluator, and TOML presets driving both systems with disk
  hot-reload. Implements **[ADR-0002](../adrs/0002-layered-preset-architecture.md) layers 1-2**
  (now **accepted**). Two review fixes at close (`6b7135b`): thread-isolated the zero-alloc test
  so both `cargo test` and nextest pass, and added `preset/expr.rs` to the hygiene guard.
  **⚠ Carry-forward (minor, non-blocking):**
  1. The three legacy scenes (spectrum/pulse/starfield) stay compiled and constructed but no
     preset addresses them - a cleanup candidate (delete, or expose via a `SystemKind`).
  2. Phase 3's iGPU 60 fps @ 1080p validation (NFR 1/9) and the Phases 1/3/5 "visibly flows and
     reacts" done-whens are runtime/hardware checks, not verifiable in review - confirm on the
     iGPU test PC when available.
  **Deferred follow-ups (tracked in the closed plan):** Rhai orchestration (layer 3),
  cross-preset blending, a compute-shader particle path for thousands-scale, additional built-in
  systems (feedback/warp, boids, walkers, 3D), and exposing preset selection across the C ABI.
- [0006 — Versioning: single source of truth + cargo-release + surfacing](done/0006-versioning-wiring.md) —
  **done 2026-07-21**, passed Mode 4 review (no blockers, no majors). Implements
  [ADR-0005](../adrs/0005-versioning-and-release-cadence.md) (now **accepted**): one
  `[workspace.package].version` inherited by both crates, `cargo-release` (`release.toml`:
  `shared-version`, tag `v{{version}}`, `push = false`, `publish = false`) as the single bump
  authority, version surfaced in the standalone title via `env!("CARGO_PKG_VERSION")`.
  Phase 4 (human) confirmed: `cargo-release 1.1.3` installed, dry-run clean. **First bump run
  at close: minor `0.1.0 -> 0.2.0`, tag `v0.2.0`, not pushed** (the user pushes). C-ABI version
  (`LMV_ABI_VERSION`) stays a separate axis; the foobar plugin version remains independent.
- [0002 — Rust enforcement tooling](done/0002-rust-enforcement-tooling.md) —
  **done 2026-07-21**, passed Mode 4 review (no blockers). Phases 0-4 landed and are green
  locally (fmt, clippy `-D warnings`, both hygiene guards, cargo-deny). Panic pragma on all 7
  core hot-path files with reasoned in-bounds escapes; no production hot-path panics.
  **⚠ Carried forward (both now tracked as their own work — no loose ends):**
  1. **Phase 5 (Miri CI job) was DEFERRED, not run** — `lmv-core`'s lib pulls the full
     wgpu/naga graph, so a full-crate Miri job is impractical (>10 min). The ring IS verified
     UB-clean locally (`cargo +nightly miri test -p lmv-core --lib`, all 5 ring tests incl. the
     cross-thread SPSC case, 95 s); only the CI automation was outstanding. **→ Now
     [Plan 0005](0005-miri-ring-extraction.md)** (draft): extract the ring into a zero-dep
     `lmv-ring` crate and run Miri against it.
  2. **Scenes were per-frame render code outside the hot-path pragma set / guard scan.** **→
     Folded into [Plan 0003](0003-generative-scenes-and-presets.md) Phase 0** (amendment):
     relocate scenes under `core/src/render/scenes/` so the guard's existing recursive `render/`
     scan covers them structurally, and add the panic pragma to each — done before 0003 fills
     `scenes/` with heavy per-frame indexing.
- [0001 — Core + standalone MVP, then foobar parity](done/0001-core-and-standalone-mvp.md) —
  **done 2026-07-21**, passed Mode 4 review (no blockers; C ABI recorded in
  [ADR-0003](../adrs/0003-c-abi-v1-surface.md)). Windows standalone + foobar2000 plugin
  smoke-tested; 9/9 tests green.
  **⚠ Carried forward: Phase 10 (macOS validation on real hardware, human) was DEFERRED, not
  run** — the plan was closed early on the user's request with the Mac path still unverified
  on-device (it compiles via CI only). When a Mac is available: run the standalone on macOS
  13+, grant the screen-recording permission, confirm live visuals; report results and route
  any fixes to `dev` (the `capture_mac` path). This is the one outstanding piece of v1.

**Open gap (from the 0001/0002 reviews):** the **C ABI has no automated test coverage** — the
C++ shim is not built in CI, and no in-crate FFI test exists. 0002 armed the pragma and
supply-chain gates but did not add an FFI test (it was never a 0002 phase). A minimal
`lmv_create`/`push`/`free` Rust-side test remains an unassigned candidate for a future plan.
Miri (the deferred 0002 Phase 5) now runs in CI via [Plan 0005](done/0005-miri-ring-extraction.md),
but **only against the ring** — the FFI `unsafe` in `core/src/ffi.rs` is renderer/window-coupled,
stays in `lmv-core`, and is out of the Miri job's scope, so the FFI pointer handling is still
uncovered (its C side remains the Plan 0001 Phase-6 smoke program's job, per ADR-0003).

## Roadmap (agreed 2026-07-21, revised same day for the live-show use case; numbers assigned when drafted)

Execution order after Plan 0001, per the NFR interviews ([docs/nfr.md](../nfr.md)):

1. **Preset / scripting engine** — layered presets per
   [ADR-0002](../adrs/0002-layered-preset-architecture.md): TOML data + expression language
   driving built-in systems (feedback/warp, boids, walkers/growth, 3D scene), with an
   optional budgeted Rhai script for staged per-track arcs (NFR §10). Replaces "scenes are
   Rust code" — Plan 0001's Scene trait becomes the rendering vocabulary presets drive, so
   keep it thin. **Delivered by [Plan 0003](done/0003-generative-scenes-and-presets.md)** (layers
   1-2: fragment-field + swarm systems, data + expression presets); Rhai (layer 3), blending, and
   compute-scale particles remain follow-ups tracked in 0003.
2. **Live performance features** — line-in/audio-interface capture, scene triggers
   (auto-rotate + hotkey/MIDI + experimental track-change detection), fullscreen on a
   chosen display/projector, 4-hour soak stability (NFR §10).
3. **Adaptive quality + runtime-memory trim** — quality tiers + frame-time governor for the
   60 fps iGPU floor (NFR §1), plus cutting the standalone's ~200 MB working set (NFR §12).
   The memory trim's primary lever — compiling wgpu with only the per-OS backend feature
   (DX12/Metal), dropping the dead Vulkan/GL paths — is a cheap, low-risk win that can
   front-run the full tier system. Both validated on the older iGPU test PC (footprint stated
   before/after; the backend trim must not regress the §1 floor).
   **Front-run by [Plan 0011](done/0011-diagnostics-and-memory-trim.md)** (diagnostics harness +
   the cheap NFR §12 levers, all-three-frontend, C ABI v3 / [ADR-0008](../adrs/0008-c-abi-v3-diagnostics.md)):
   it builds the before/after measuring stick and lands the wgpu-backend + swapchain trims. The
   **adaptive-quality tiers + frame-time governor remain** for a later plan — 0011 explicitly
   does not do them.
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
