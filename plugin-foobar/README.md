# plugin-foobar

The foobar2000 visualization component: a thin C++ shim over lmv-core's C ABI
(`core/include/lmv_core.h`). Windows-only per ADR-0001.

## SDK location (Plan 0001 Phase 7)

The foobar2000 SDK (release **2025-03-07**) is unpacked at `plugin-foobar/sdk/`:

```
plugin-foobar/sdk/
├── foobar2000/       # SDK proper + component client + helpers
├── pfc/              # foundation classes the SDK depends on
├── libPPUI/          # UI helpers (unused by this component)
└── sdk-license.txt
```

The SDK is third-party and separately licensed — it is gitignored, never
committed. To recreate: download from <https://www.foobar2000.org/SDK> and
extract the archive to `plugin-foobar/sdk/`.

Toolchain: MSVC (VS Build Tools 2022, x64).
