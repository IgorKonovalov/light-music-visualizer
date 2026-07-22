# Architecture Decision Records

Numbered, append-only records of decisions that have a rejected alternative worth
remembering. Accepted ADRs are never edited in place — to change a decision, write a new
ADR that supersedes the old one and update the status here.

Rule of thumb: if you can't name an option you're *not* taking, you don't need an ADR —
you need a code comment.

**Next free number: 0016**

| ADR  | Title                                                      | Status   |
|------|------------------------------------------------------------|----------|
| [0001](0001-rust-core-wgpu-cabi-foobar-shim.md) | Rust core, wgpu rendering, C ABI with a C++ foobar shim | accepted |
| [0002](0002-layered-preset-architecture.md) | Layered preset architecture: data + expressions + optional script | accepted |
| [0003](0003-c-abi-v1-surface.md) | C ABI v1 surface (eight functions; frozen shape + rationale) | accepted (extended by 0006, 0008) |
| [0004](0004-living-behavioral-spec-layer.md) | Living behavioral-spec layer: seed two contracts, no gate/ritual yet | accepted |
| [0005](0005-versioning-and-release-cadence.md) | App versioning: SemVer 0.x, one workspace version, cargo-release at plan close | accepted |
| [0006](0006-c-abi-v2-preset-loading.md) | C ABI v2: add lmv_load_presets (seed-then-load); bump to v2 | accepted |
| [0007](0007-line-geometry-generators.md) | Line-geometry generators: cached-build built-in category + instanced-quad line rendering | proposed |
| [0008](0008-c-abi-v3-diagnostics.md) | C ABI v3: diagnostics query (lmv_get_metrics) + debug-overlay toggle (lmv_set_debug); bump to v3 | accepted |
| [0009](0009-glyphon-text-rendering.md) | Adopt glyphon for standalone on-canvas text (feature-gated) | accepted |
| [0010](0010-accept-gpu-driver-memory-floor.md) | Accept the DX12/wgpu driver-stack memory floor; retarget the runtime-memory NFR (§12) | accepted |
| [0011](0011-image-crate-for-capture-tooling.md) | Use the `image` crate (dev-dependency only) for headless-capture PNG I/O and golden compare | accepted |
| [0012](0012-stateful-feedback-render-system.md) | Stateful feedback render system: ping-pong offscreen simulation + fixed-timestep accumulator (Gray-Scott first) | proposed |
| [0013](0013-c-abi-v4-render-dt.md) | C ABI v4: add lmv_render_dt (injected real dt); bump to v4 | proposed |
| [0014](0014-preset-dir-override-for-dev-iteration.md) | Preset-directory override (`LMV_PRESET_DIR`) with a shared resolver, polling over a watcher | proposed |
| [0015](0015-gpu-compute-particle-idiom.md) | GPU compute pipelines for particle scenes; the four render-idiom catalogue (attractors first) | proposed |
