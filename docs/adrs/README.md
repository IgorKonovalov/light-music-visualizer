# Architecture Decision Records

Numbered, append-only records of decisions that have a rejected alternative worth
remembering. Accepted ADRs are never edited in place — to change a decision, write a new
ADR that supersedes the old one and update the status here.

Rule of thumb: if you can't name an option you're *not* taking, you don't need an ADR —
you need a code comment.

**Next free number: 0007**

| ADR  | Title                                                      | Status   |
|------|------------------------------------------------------------|----------|
| [0001](0001-rust-core-wgpu-cabi-foobar-shim.md) | Rust core, wgpu rendering, C ABI with a C++ foobar shim | accepted |
| [0002](0002-layered-preset-architecture.md) | Layered preset architecture: data + expressions + optional script | accepted |
| [0003](0003-c-abi-v1-surface.md) | C ABI v1 surface (eight functions; frozen shape + rationale) | accepted (extended by 0006) |
| [0004](0004-living-behavioral-spec-layer.md) | Living behavioral-spec layer: seed two contracts, no gate/ritual yet | accepted |
| [0005](0005-versioning-and-release-cadence.md) | App versioning: SemVer 0.x, one workspace version, cargo-release at plan close | accepted |
| [0006](0006-c-abi-v2-preset-loading.md) | C ABI v2: add lmv_load_presets (seed-then-load); bump to v2 | proposed |
