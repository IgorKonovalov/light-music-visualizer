# Plans index

The one-minute "what's in flight" view. Read this first each session instead of
re-deriving state from `git log`. Completed plans move to `done/`.

**Next free number: 0021**

## Active roster

| Plan | Title                                   | Status | Summary |
|------|-----------------------------------------|--------|---------|
| [0015](0015-preset-dir-override-and-live-iteration.md) | Preset-directory override + live iteration (`LMV_PRESET_DIR`, shot flags) | approved | Edit one **version-controlled** `presets/*.toml` and see it live in **both** the running standalone app and the headless `shot` CLI, no rebuild. An `LMV_PRESET_DIR` env var (honored by both Rust frontends via a **single shared resolver** extracted into a `standalone` lib module) overrides the seeded `%APPDATA%` dir; the app hot-reloads it on a tightened ~150 ms poll (skipping seeding when overridden), and `shot` gets `--presets <dir>` / `--preset-file <path>` flags that beat the env var. Dependency-free (polling, **no** `notify` crate); framed as a power-user "custom preset folder" knob; consolidates Plan 0007's duplicated Rust resolver. Standalone + docs only, **C ABI untouched (v3)**; foobar plugin out of scope. [ADR-0014](../adrs/0014-preset-dir-override-for-dev-iteration.md); rejected app CLI args, symlink, `notify` watcher, duplicated resolver, core-side resolution. |
| [0016](0016-gpu-compute-particle-scenes.md) | GPU compute-particle scenes: strange attractors | approved | The engine's first **GPU compute-particle** family and idiom B from [ADR-0015](../adrs/0015-gpu-compute-particle-idiom.md): a compute shader steps a storage buffer of particles through a strange-attractor map (De Jong, Clifford, Thomas, 3D-projected Lorenz) each frame from injected `dt`, drawn as additive point-sprites with trails via Plan 0014's `PingPongField`. Attractor coefficients + look scalars are ADR-0002 layer-2 named params so presets bind them to bands/beat; init is `SeededRng`-seeded for determinism (NFR §6). Replaces the CPU swarm's ~10k ceiling for the dense glowing-point look; scales to 100k+ GPU-resident. **First compute pipeline in core** (the ADR's crux); rejected fragment/texture-state particles and extending the CPU swarm. Core-only, **C ABI untouched**, no new dependency. Curl-noise flow fields, fractal flames, and boids are follow-ups on the same idiom. Sequenced **after 0014** (reuses its `PingPongField` + injected-`dt` clock). |
| [0018](0018-engine-wide-visual-enrichment.md) | Engine-wide visual enrichment: zoom, atmosphere, easing, mirrors | approved | Add four engine-wide visual controls the live smoke of the line scenes surfaced as missing: a shared **view transform** (zoom/pan), a **gradient/atmosphere background** behind every scene, frame-rate-independent **parameter easing** (band/beat motion stops feeling rigid and fast), and **mirrors** — a true line-geometry fractal replication *and* a screen-space kaleidoscope. Lands as a **fixed-order engine composite** (background → scene+view → trails → kaleidoscope → present, [ADR-0018](../adrs/0018-engine-wide-scene-compositing.md)) plus a **render-layer smoothing seam** ([ADR-0019](../adrs/0019-eased-parameters.md)), all audio-bindable as ADR-0002 named params. **Sequenced after 0014** (reuses its offscreen/present + `PingPongField` + injected `dt`); Phases 1-4 (view/background/geometry-mirror) are 0014-independent, Phases 5-7 (easing/trails/kaleidoscope) need it. Core-only, **C ABI untouched**, no new dependency. Rejected a general render graph, per-scene effects, screen-space-only mirror, a stateful `smooth()` builtin, and smoothing on the fixed 1/60 clock. |
| [0019](0019-preset-grammar-v2.md) | Preset expression grammar v2: branching, math functions, tempo, typo warnings | approved | Grow the preset expression language so authors stop hitting walls: add math functions (`cos sqrt pow smoothstep mod`) + constants (`pi tau`), branching (six comparison operators yielding `0/1` + a `select(cond,x,y)` conditional), and two new variables — `tempo` and experimental `novelty` (plumb the already-computed `AnalysisFrame.bpm`/`novelty`, no new DSP). Fix the silent-typo footgun — an unknown parameter name becomes a surfaced load **warning** (preset still loads its good bindings), backed by each system declaring its param vocabulary. Rewrite the stale `docs/presets.md` (claims 10 presets / 2 systems; code ships 17 / 5) last. Core-only, allocation-free/panic-free per frame, **C ABI untouched**; no new DSP, no boolean ops, no ternary, no Rhai. Pre-1.0 so no backward-compat obligation (additions are incidentally non-breaking). [ADR-0020](../adrs/0020-preset-grammar-v2-branching-functions-tempo.md), supplements [ADR-0002](../adrs/0002-layered-preset-architecture.md); from `preset-author`-lane grammar feedback. |
| [0020](0020-shared-palette-system.md) | Shared palette system: gradient LUT, named + custom palettes, bindable color | draft | Give presets real color control. Both preset-driven scenes hardcode the **same** iq cosine palette, so a preset's only color lever is a scalar `hue` offset — fragment fields can't hold a cohesive mood and swarm color is unreachable (per-particle hue is random across the full wheel, making `hue` a visual no-op). Add one shared `core/src/render/palette.rs` that bakes a gradient (built-in **named** palette or **custom stops**) into a 256-entry LUT every scene colors through — delivered to the fragment scene as a 256x1 1D texture, sampled on CPU by the swarm — plus **fully bindable** color via layer-2 named params (`saturation`, `hue`, fragment `color_span`/`color_center`, swarm `hue_spread`/`hue_center`) and an A/B `palette_mix` crossfade for bindable palette *selection*. Default `spectrum` palette = the exact current cosine, so the 17 shipped presets are **visually unchanged** until re-authored. Adds one thin off-hot-path `Scene::set_palette` hook (second widening after ADR-0007's `configure`). Core-only, **C ABI frozen**, no new dependency; from `preset-author`-lane color feedback (commit 76a2fb4). [ADR-0021](../adrs/0021-shared-palette-system.md), supplements [ADR-0002](../adrs/0002-layered-preset-architecture.md); rejected in-shader stop-array, cosine-coefficient params, minimal per-scene fix, bindable integer palette index, OKLab color management. **Independent of the `dt` coupling** (touches preset schema + scene shaders, not the render clock) — can land anytime. |
| [0014](0014-reaction-diffusion-feedback-scene.md) | Reaction-diffusion feedback scene + frame-rate-independent render clock | approved | The engine's first **stateful feedback** scene: a Gray-Scott reaction-diffusion simulation (evolving nested contours / cellular tissue / hatched restructuring maze) on a new reusable `render::feedback::PingPongField` (two offscreen `Rgba16Float` textures swapped each sub-step, fixed internal grid, present pass composites to the surface). Driven by a **fixed-timestep accumulator fed by real injected `dt`** so it looks identical on any device over wall-clock time — the core stays clock-free (Plan 0013 capture feeds a fixed `dt` for reproducibility). Delivering `dt` at the render seam (`Renderer::render(&frame, dt)`) lets us **converge the shared scene clock globally and retire `SCENE_DT`**, making every existing scene frame-rate-independent (resolves the standing SCENE_DT wish). Audio drives it via existing named params (ADR-0002 layer 2): bands modulate feed/kill/flow, beats stamp seeds. Adds C ABI **v4 `lmv_render_dt`** ([ADR-0013](../adrs/0013-c-abi-v4-render-dt.md), additive; `lmv_render` becomes the 1/60 wrapper) so the foobar plugin gets parity. New feedback render system per [ADR-0012](../adrs/0012-stateful-feedback-render-system.md); rejected warp-feedback advection, engine-managed multi-pass, per-frame stepping. Core + both frontends. Cross-plan dep: 0013's capture must thread a fixed `dt`. |

## Recommended execution sequence

A tactical ordering of the **active roster** (strategic themes live in the Roadmap below). One
coupling drives it: **0014 depends on 0013's capture threading a fixed `dt`** while it retires
`SCENE_DT`. (**Plan 0013 has now landed and closed** — the capture/visual-QA harness is available;
`Renderer::capture_preset`/`capture_audio` + the `shot` CLI + the `core/tests/` reactivity/
animation/sanity/beat/golden suite exist for the scene plans below to build against.)

1. **[0014] Reaction-diffusion feedback + retire `SCENE_DT`** — the harness is now in place to test
   the feedback scene. **0014 should own the `SCENE_DT` → injected-`dt` migration**, updating
   0013's now-landed `capture_preset`/`capture_audio` to pass a fixed `dt`, so the change lives in
   one plan. Adds C ABI v4 ([ADR-0013](../adrs/0013-c-abi-v4-render-dt.md)). *Approved — its scope
   should update 0013's capture primitives to the injected `dt` before dev starts either plan.*
2. **[0016] GPU compute-particle scenes (attractors)** — sequenced **after 0014**: it reuses 0014's
   `PingPongField` (trails) and injected-`dt` clock, so 0014 must land first. Introduces the first
   compute pipeline in core ([ADR-0015](../adrs/0015-gpu-compute-particle-idiom.md)). **Approved** —
   ready for `dev` once 0014 has landed.
3. **[0018] Engine-wide visual enrichment (zoom/atmosphere/easing/mirrors)** — also sequenced
   **after 0014**: Phases 5-7 (easing, trails, screen-space kaleidoscope) reuse 0014's injected
   `dt` + offscreen/present + `PingPongField`, though Phases 1-4 (view transform, background,
   geometry mirror) are 0014-independent. Engine-wide composite + easing seam
   ([ADR-0018](../adrs/0018-engine-wide-scene-compositing.md), [ADR-0019](../adrs/0019-eased-parameters.md));
   C ABI untouched. **Approved** — ready for `dev` once 0014 has landed. A natural companion to
   0016 (both build the post-0014 composite/effects layer).

(**[0010] Line-geometry scenes has now landed and closed** — the line-art category (roses,
L-systems, Hankin stars) built on the shared `LineRenderer` is available; see Recently closed.)

**[0015] Preset-dir override** is small, standalone + docs only, and **independent of the coupling
above** (no `core` change, C ABI frozen) — it can land anytime. It's a strong companion to the
scene plans (0010/0014): the shared-preset iteration loop it adds — edit `presets/*.toml`, see it
live in the app and `shot` — is exactly how a developer/agent will tune the new curves and feedback
presets those plans introduce, so doing it **before or alongside** 0010/0014 pays for itself.

**[0019] Preset grammar v2** is likewise **independent of the `dt` coupling** (it touches `preset/expr.rs`
+ `schema.rs` + the load path, not the render clock or any scene), so it can land anytime. It's a natural
companion to **[0015]** and the `preset-author` lane: 0015 gives the fast edit loop, 0019 widens what a
preset can express (real `cos`, easing, `tempo`, threshold `select`) and stops the silent-typo footgun —
so pairing them sharpens preset authoring end to end. Small, core-only, C ABI frozen. **Approved** —
ready for `dev` (no ordering dependency; can land before or alongside the scene plans).

**[0020] Shared palette system** is also **independent of the `dt` coupling** (it touches the preset
schema + scene shaders + a new `render/palette.rs`, not the render clock or any feedback state), so it
can land anytime. It completes the preset-color axis the way 0019 completes the expression axis — a
natural companion to **[0015]** (fast edit loop) and **[0019]** (grammar), and to the `preset-author`
lane whose color feedback (commit 76a2fb4) motivated it. Core-only, C ABI frozen, no new dependency.
**Draft** — needs a plan walkthrough/approval before `dev` (unlike 0015/0016/0018/0019, which are
approved). [ADR-0021](../adrs/0021-shared-palette-system.md).

## Standing (not a plan)

- **[On-device validation — low-end Windows iGPU smoke](../on-device-validation.md)** — a
  hardware-gated checklist, **not** a phased plan and **not** in the roster above: it never blocks a
  plan from closing. Holds the low-end / older Windows iGPU checks (fps floor ≥ 60 @ 1080p; footprint
  on a second GPU vendor) the user can only run once that box is in hand. Ticked when run; deleted when
  empty. Currently home to the extracted Plan 0012 Phase 3 (also covers the identical Plan 0003 Phase 3
  iGPU-fps carry-forward).

## Recently closed

- [0009 — Live performance features (standalone)](done/0009-live-performance-features.md) —
  **done 2026-07-23**, passed Mode 4 review (no blockers, no majors; two minor deviations, both
  pre-flagged and reconciled). Five `dev` phase commits (`6e048d0` per-user config + borderless-
  fullscreen on a chosen display, `3891272` line-in / audio-interface capture selection, `bb9a1e2`
  drop-biased scene director + hotkeys, `d693c69` experimental track-change novelty nudge, `d49f377`
  `--soak` long-run instrumentation). Made the standalone drive a live DJ show: `Fullscreen::
  Borderless` on the config-selected display (`F`/`D` hotkeys, name-over-index monitor match),
  WASAPI **capture**-endpoint enumeration for line-in alongside loopback (`--list-devices`, graceful
  default fallback), a clock-free **`director`** module (MilkDrop-style dwell timer, drop bias, `A`
  toggle, manual `Space`, all a pure function of injected `dt` + `AnalysisFrame`), and a coarse soak
  sampler (elapsed/fps/RSS/heartbeat every 5 s, off the per-frame path). Core gained **one
  deterministic scalar** — `AnalysisFrame.novelty` from a new `dsp/novelty.rs` (normalized spectral
  distance from a slow running mean; pure, seeded, hot-path pragma) — consumed via the native API as
  a director *nudge* that never triggers alone. Operator choices persist in a per-user `config.toml`.
  **C ABI untouched (still v3), no ADR** (standalone-only; ADR-0001 layering applied). Verified:
  `cargo test -p lmv-core` green (incl. `novelty_spikes_at_a_spectral_boundary` + determinism
  extended to `novelty` bits); 11 `director` unit tests green (timing/bias, drop/novelty-before-min
  holds, steady-never-rotates-on-novelty, disabled-nudge, inverted-dwell clamp); `cargo clippy
  -p lmv-core -p standalone --all-targets -D warnings` clean; hygiene guard covers `dsp/novelty.rs`
  (recursive `dsp/` scan + pragma); `ffi.rs`/`ffi` tests zero-diff. **Minor (reconciled):** (1)
  `config.rs` edited in Phase 2 though its file list omitted it — the `[input]` schema was a genuine
  prerequisite (flagged in the commit body); (2) novelty is **spectral-only**, not the plan's
  "spectral/tempo" — a deliberate narrowing (a beatmatched set holds tempo across the blend, so a
  tempo term would fire on exactly the case the nudge must stay soft on; rationale in `novelty.rs`),
  fully satisfying the distinct-spectra done-when. **⚠ Carry-forwards (on-device, non-blocking, like
  prior plans):** Phase 6 (≥4-hour projector-rig soak, human); Phase 1 multi-monitor fullscreen +
  `F`/`D` + persistence-across-restart + config-delete fallback; Phase 2 line-in reactivity with an
  interface connected (loopback + `--list-devices` smoke-verified live); auto-rotate "feel" tuning
  (`NOVELTY_REF`, dwell/drop constants intentionally in code/config for on-rig calibration).
  Delivers roadmap item 2 (live performance features). Version **minor 0.3.1 → 0.4.0** at close.

- [0017 — Green CI: reasoned ttf-parser advisory ignore + adapter-skip for headless GPU tests](done/0017-ci-green-advisory-and-gpu-tests.md) —
  **done 2026-07-23**, passed Mode 4 review (no blockers, no majors, no minors; two non-actionable
  nits). Two `dev` phase commits (`95bf510`, `134d4e3`) unbreaking `main` (CI run 29985131075) after
  two **environmental** failures. **Phase 1** silenced `RUSTSEC-2026-0192` — `ttf-parser` flagged
  **unmaintained** (not a vulnerability), load-bearing via the glyphon text stack (`ttf-parser →
  fontdb → cosmic-text → glyphon → lmv-core`, both shipped targets, ADR-0009's `text` feature),
  upstream-pinned so undroppable without reversing ADR-0009 or waiting on upstream — with a single
  **reasoned, tracked `advisories.ignore`** entry (id + unmaintained-not-vuln framing + load-bearing
  path + revisit trigger) and corrected the now-false `deny.toml` `[graph]` pruning comment. **Phase 2**
  routed the three headless GPU-capture tests through a shared `headless_or_skip` helper that **skips
  on `RenderError::RequestAdapter`** (macOS lost its Metal adapter; no software fallback) and panics on
  any other build error, per [ADR-0016](../adrs/0016-gpu-tests-opt-in-ci-scope.md) (now **accepted**) —
  keeping full assertions running on Windows WARP every push. Verified: `cargo deny check` exits 0
  (`advisories/bans/licenses/sources ok`); all three tests run their full assertions green on WARP under
  nextest; clippy `-D warnings --all-targets` clean with the production hot-path `#![deny(clippy::panic)]`
  intact (the `clippy::panic` allow is scoped to the `#[cfg(test)]` module only). **Config + test-only —
  no `ci.yml` change, C ABI untouched (still v3), no hot-path surface.** **Followup (standing):** remove
  the ignore once a glyphon/cosmic-text bump no longer pins the unmaintained ttf-parser. **Nits
  (non-actionable):** the `skipped:` notice is stderr-only (nextest hides it on pass) — the exact
  silent-no-op tradeoff ADR-0016 accepts; and the parallel session's untracked `skill-creator/` /
  `skills-lock.json` were surfaced-not-swept.

- [0010 — Line-geometry scenes: parametric curves, L-systems, star patterns](done/0010-line-geometry-scenes.md) —
  **done 2026-07-23**, passed Mode 4 review (no blockers, no majors; three minor, two nits). Five
  `dev` phase commits (`110eab7`, `cd0e518`, `4b9ea05`, `1cc7fa1`, `3e2dcc1`) implementing
  [ADR-0007](../adrs/0007-line-geometry-generators.md) (now **accepted**). Added a **line-art
  category** to the built-in vocabulary on one shared `LineRenderer` (segments → thick glowing
  instanced quads, additive blend, fixed 20k-segment buffer) under two build models: a **parametric**
  system (`ParametricCurveScene`, the Maurer rose sampled per frame, allocation-free into a
  preallocated buffer) and a **generator** system built + cached at preset load
  (`LSystemScene`: grammar rewrite + turtle-walk cached per depth `1..=max_depth`; `StarPatternScene`:
  Hankin contact-angle rosette with precomputed variants). The `Scene` trait grew by **exactly one**
  optional off-hot-path method (`configure(&GeneratorConfig)`, default no-op) — the single widening
  ADR-0007 sanctions — invoked once at preset load from `configure_active_scene`. New optional
  `[curve]`/`[generator]` TOML tables validated at the load boundary (`schema.rs`), 7 curated presets
  (4 roses, 2 L-systems, 1 star) embedded + seeded, and a `presets/README.md` authoring note.
  Verified: `cargo test -p lmv-core` green — grammar exact-string (incl. Koch depth-2), turtle
  cap-truncates-and-reports, Hankin segment-count + 2π/n rotational symmetry, zero-per-frame-alloc,
  and bad-`[curve]`/`[generator]`-config rejection, all present and non-tautological; clippy
  `-D warnings` clean; hygiene guard covers all nine new `lines/*.rs` panic pragmas. **C ABI untouched
  (still v3).** **⚠ Carry-forward (minor, non-blocking):** (1) `LSystemScene::overflow()` and the
  parametric `samples` clamp *track* the segment-cap drop but nothing *surfaces* it at load — the
  plan's "never a silent cut" is unmet in the surfacing half (latent: shipped fern peaks ~6k/20k).
  (2) `presets/README.md` says `max_depth` is "clamped to 1..=7" but schema rejects `>7` as a load
  error. (3) `parametric` `configure` is skipped when `[curve]` is omitted, so `family` doesn't reset
  (harmless with one family). (4) `lsystem_fern` `visible_depth` only bumps depth at `bass == 1.0`
  exactly — an on-device tuning nit. The iGPU 60 fps @ 1080p confirmation is the standing hardware
  carry-forward (`docs/on-device-validation.md`).

- [0013 — Headless scene capture + differential visual QA + golden images + shot CLI](done/0013-headless-scene-capture.md) —
  **done 2026-07-22**, passed Mode 4 review (no blockers, no majors; one minor, one nit). Eight
  `dev` phase commits (`ecc50e5`, `ba68026`, `d11a7f0`, `889f4e3`, `26a3180`, `4b54d1e`, `8152943`,
  `4364464`) plus the `assets/test` gitignore (`a16be92`). Gave the agent a **windowless
  visual-feedback + QA harness**: `RenderContext::new_headless` (a surface-less device+queue, `None`
  surface so the on-surface present path is byte-unchanged) + a shared `draw_frame` extracted from
  `render`, feeding an offscreen `render/capture.rs` (clear-to-black → draw → 256-byte-aligned
  `copy_texture_to_buffer` → blocking map → tight `CaptureImage`). Two pure primitives —
  `capture_preset(name, frame, N)` (constant synthetic frame) and `capture_audio(name, pcm, fmt,
  at[])` (feeds PCM hop-by-hop through the **real** `dsp::Analyzer`, format validated at the intake
  boundary — source-agnostic rule preserved), each rebuilding scenes to their seed + resetting the
  clock so a capture is a pure function of its inputs. A dependency-free `render/metrics.rs`
  (`frame_diff`, recolor-robust Sobel `struct_diff`, `coverage`, `quadrant_spread`) powers **hard
  `core` tests**: per-band `reactivity`, `animation` (N vs N+k), `sanity` (coverage+spread against
  each scene's own sampled background — not tautological), and `beat` (a `core::signal` 120 BPM
  click track through the real DSP; a zeroed-beat-binding probe stays below the floor). Plus an
  **advisory** dual-metric `distinctness` report, `golden`-drift regression (software adapter +
  mean/outlier tolerance + `LMV_BLESS`, three eyeballed baselines), and a `standalone/examples/shot.rs`
  CLI (`--preset/--set/--frames/--size/--out`, `--all` contact sheet, `--report [--json]`,
  `--signal`/`--audio --strip` filmstrips via a hand-rolled 16-bit-PCM WAV reader). `image` is a
  **dev-dependency** in both crates ([ADR-0011](../adrs/0011-image-crate-for-capture-tooling.md), now
  **accepted**); the audio path adds **no** dependency; **C ABI untouched (still v3)**. Verified:
  full `cargo test -p lmv-core` green (18 lib unit + 8 integration binaries), `cargo clippy
  -p lmv-core -p standalone --all-targets -D warnings` clean (lints the example), hygiene guard
  covers the new `render/` files' panic pragma + the `image` exact-pin (`=0.25.10`). **⚠ Phase 9
  (human) outstanding — non-blocking:** the CC0 demo clip. Dev implemented a **safer variant** —
  `assets/test/*` is **gitignored** (tracked README only), so a clip is supplied and used *locally,
  never committed*; the `--signal` path already validates the whole audio pipeline with no asset.
  **Minor:** that gitignore supersedes Phase 9's literal "commit a WAV under `assets/test/`"
  done-when (reconciled in the closed plan). **Nit:** `core/src/signal.rs` is a new top-level core
  module outside the hygiene panic-pragma scan set and carries no pragma — acceptable, since it runs
  at capture-setup time (not per-frame or in the audio callback) and is written slice-index-free, so
  it's within the plan's own "only if it carries per-frame indexing" guidance.

- [0008 — In-app preset browse overlay (standalone)](done/0008-preset-browse-overlay.md) —
  **done 2026-07-22**, passed Mode 4 review (no blockers, no majors). Four `dev` phase commits
  (`3bef1a8`, `b0bb95e`, `43f3b39`, `9cc3234`). Landed the codebase's first text rendering:
  **glyphon** behind a non-default core `text` feature ([ADR-0009](../adrs/0009-glyphon-text-rendering.md),
  now **accepted**) via a reusable `render::text::TextLayer` seam (a second load-pass compositing
  positioned `TextRun`s over the scene in one frame; Plan 0009's HUD reuses it). The standalone draws
  the active preset name on-canvas, plus a Tab-toggled browse overlay: pure window-free `OverlayState`
  (open/highlight/filter → `OverlayAction`) with arrow-nav, case-insensitive substring type-to-filter,
  Enter selecting the **absolute** roster index, Esc closing, and hot-reload re-clamp. Selection landed
  as `Renderer::preset_names`/`select_preset`, both delegating 1:1 to a new crate-private, unit-tested
  `Roster` (the surface-free selection state — mirrors the FrameStats-behind-Diag pattern, since a live
  `Renderer` can't be built headlessly). **C ABI untouched** (`LMV_ABI_VERSION` stays 2); the plugin
  stays cycle-only. Verified: 41/41 tests (3 `Roster` + 11 `OverlayState`, incl. the absolute-index
  and no-wrap asserts) green under nextest; clippy `-D warnings` clean on the `--features text` build;
  `cargo tree` confirms glyphon absent from the default/plugin graph, present only in the standalone;
  hygiene guard covers `text.rs`'s panic pragma and the glyphon exact-pin. **⚠ Carry-forward (human,
  on-box):** Phases 1/3/4 visual done-whens ("legible on canvas", "switches the visual", "narrows
  live") and the NFR 4 binary-size delta (release `lmv.exe` ≈ 7.6 MB with glyphon; the pre-glyphon
  delta wasn't isolated — the standalone hard-enables `text`) are GPU/on-device judgments.
  **Follow-up (not blocking):** a `--features text` core build check in CI (the two-shape build's green
  is a local gate only this session), alongside the standing FFI/Miri CI notes. **Minor:** the browse
  overlay's type-to-filter drops whitespace, so a preset name containing a space can't be fully matched
  (Plan 0007's "filenames only" makes this latent, not live).

- [0012 — Measure the driver-memory floor + cull dead scenes](done/0012-memory-floor-measure-and-scene-cull.md) —
  **done 2026-07-22**, passed Mode 4 review (no blockers, no majors). Two `dev` phase commits
  (`50a7ea0`, `3de5611`); the third phase (human, low-end iGPU) was **extracted** to the standing
  `docs/on-device-validation.md` checklist so the plan could close on completed work rather than wait on
  hardware. **Phase 1** culled the three dead legacy scenes (`spectrum`/`pulse`/`starfield` — built +
  driver-compiled at startup, addressed by no preset; closes the Plan 0003 carry-forward), leaving
  `scenes/mod.rs` at `fragment_field` + `swarm`; measured delta **WS −3.3 MB / private −2.0 MB**, first
  data point that pipeline count is a weak memory lever. **Phase 2** stood up `standalone/examples/floor.rs`,
  a throwaway scene-less wgpu-context spike (construct-only — `RenderContext` exposes only
  `new`/`resize`/`surface_format`, so the example measures at the configure boundary without widening
  core's surface), isolating the fixed driver floor: **~327 MB private commit vs ~338 MB post-cull
  standalone → our whole visual system is only ~11 MB (~3%)**. This resolves ADR-0010's two open items
  (floor-vs-overhead split; pipeline count as a real-but-weak lever) and gave NFR §12 the hard
  per-system denominator it lacked (folded in at close). Verified: `cargo test -p lmv-core` 9/9,
  `cargo clippy --workspace --all-targets -D warnings` clean (lints the example too), no dangling refs
  to the deleted modules, `floor.rs` links into no shipped binary and adds no dependency, dev-box smoke
  rendered all 10 presets at ~165 fps / 0 drops. Core-internal + throwaway example; **C ABI frozen, no
  new ADR.** **⚠ Carry-forward (human):** the low-end iGPU / second-vendor capture — see
  `docs/on-device-validation.md` (does not block anything).

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
   **Delivered by [Plan 0009](done/0009-live-performance-features.md)** (standalone borderless-
   fullscreen on a chosen display, line-in capture selection, drop-biased scene director +
   hotkeys, spectral track-change novelty nudge on the native `Frame`, `--soak` instrumentation;
   C ABI frozen). **MIDI triggers and the ≥4-hour projector-rig soak run remain** — MIDI is its
   own ADR-backed follow-up; the soak run is a `human` on-device carry-forward.
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
