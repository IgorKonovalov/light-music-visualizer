# Architecture Decision Records

Numbered, append-only records of decisions that have a rejected alternative worth
remembering. Accepted ADRs are never edited in place — to change a decision, write a new
ADR that supersedes the old one and update the status here.

Rule of thumb: if you can't name an option you're *not* taking, you don't need an ADR —
you need a code comment.

**Next free number: 0026**

| ADR  | Title                                                      | Status   |
|------|------------------------------------------------------------|----------|
| [0001](0001-rust-core-wgpu-cabi-foobar-shim.md) | Rust core, wgpu rendering, C ABI with a C++ foobar shim | accepted |
| [0002](0002-layered-preset-architecture.md) | Layered preset architecture: data + expressions + optional script | accepted (supplemented by 0020) |
| [0003](0003-c-abi-v1-surface.md) | C ABI v1 surface (eight functions; frozen shape + rationale) | accepted (extended by 0006, 0008) |
| [0004](0004-living-behavioral-spec-layer.md) | Living behavioral-spec layer: seed two contracts, no gate/ritual yet | accepted |
| [0005](0005-versioning-and-release-cadence.md) | App versioning: SemVer 0.x, one workspace version, cargo-release at plan close | accepted |
| [0006](0006-c-abi-v2-preset-loading.md) | C ABI v2: add lmv_load_presets (seed-then-load); bump to v2 | accepted |
| [0007](0007-line-geometry-generators.md) | Line-geometry generators: cached-build built-in category + instanced-quad line rendering | accepted |
| [0008](0008-c-abi-v3-diagnostics.md) | C ABI v3: diagnostics query (lmv_get_metrics) + debug-overlay toggle (lmv_set_debug); bump to v3 | accepted |
| [0009](0009-glyphon-text-rendering.md) | Adopt glyphon for standalone on-canvas text (feature-gated) | accepted |
| [0010](0010-accept-gpu-driver-memory-floor.md) | Accept the DX12/wgpu driver-stack memory floor; retarget the runtime-memory NFR (§12) | accepted |
| [0011](0011-image-crate-for-capture-tooling.md) | Use the `image` crate (dev-dependency only) for headless-capture PNG I/O and golden compare | accepted |
| [0012](0012-stateful-feedback-render-system.md) | Stateful feedback render system: ping-pong offscreen simulation + fixed-timestep accumulator (Gray-Scott first) | accepted |
| [0013](0013-c-abi-v4-render-dt.md) | C ABI v4: add lmv_render_dt (injected real dt); bump to v4 | accepted |
| [0014](0014-preset-dir-override-for-dev-iteration.md) | Preset-directory override (`LMV_PRESET_DIR`) with a shared resolver, polling over a watcher | proposed |
| [0015](0015-gpu-compute-particle-idiom.md) | GPU compute pipelines for particle scenes; the four render-idiom catalogue (attractors first) | accepted |
| [0016](0016-gpu-tests-opt-in-ci-scope.md) | Headless GPU-capture tests skip when no adapter is present (keep GPU out of the CI contract) | accepted |
| [0017](0017-preset-author-skill-lane.md) | A third skill lane: `preset-author` (preset content, not engine code); two-skill harness becomes three | accepted |
| [0018](0018-engine-wide-scene-compositing.md) | Engine-wide scene compositing: shared view transform + background pre-pass + feedback trails + screen-space post-effects (fixed order, not a render graph) | proposed |
| [0019](0019-eased-parameters.md) | Eased (smoothed) parameters: render-layer one-pole filtering on injected `dt`; expression layer stays pure | proposed |
| [0020](0020-preset-grammar-v2-branching-functions-tempo.md) | Preset expression grammar v2: branching (compares + `select`), math functions, `tempo` variable, soft typo warnings (supplements 0002) | proposed |
| [0021](0021-shared-palette-system.md) | Shared preset-controllable palette system: baked gradient LUT (named + custom stops), bindable color modulation + A/B crossfade (supplements 0002) | proposed |
| [0022](0022-build-time-preset-embedding.md) | Build-time embedding of the preset library: zero-dep `core/build.rs` generates `EMBEDDED` from `presets/*.toml` (drop a file, it ships; no code edit) | accepted |
| [0023](0023-golden-drift-guard-uses-frozen-fixtures.md) | Golden drift guard renders frozen per-system test fixtures (exhaustive `match SystemKind`), not shipped presets; shipped presets keep only behavioral floors | accepted |
| [0024](0024-cross-preset-transitions.md) | Cross-preset transitions: two-input blend stage over the engine composite, adaptive dual-live/freeze, engine-default policy (builds on 0018) | proposed |
| [0025](0025-foobar-component-version-single-sourced.md) | Single-source the foobar component version from the workspace version via a build-time generated header (revises Plan 0006's independent-plugin-version note); C ABI axis untouched | proposed |
