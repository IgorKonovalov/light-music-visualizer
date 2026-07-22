# 0015 — Preset-directory override + live iteration (`LMV_PRESET_DIR`, shared resolver, shot flags)

> **Status:** approved
> **Created:** 2026-07-22
> **Owner skill(s):** dev
> **Related ADRs:** [0014](../adrs/0014-preset-dir-override-for-dev-iteration.md) (the override
> mechanism + rejected alternatives), [0002](../adrs/0002-layered-preset-architecture.md) (the
> preset layer being edited)
> **Related plans:** [0007](done/0007-curated-preset-library.md) (seed-to-`%APPDATA%` + the
> duplicated resolver this consolidates), [0013](done/0013-headless-scene-capture.md) (the `shot`
> CLI these flags extend)

## TL;DR

Let a developer or agent **edit one version-controlled `presets/*.toml` file and see it live in
both the running standalone app and the headless `shot` CLI**, with no rebuild. An
`LMV_PRESET_DIR` environment variable, honored by both Rust frontends via a **single shared
resolver** (extracted into a `standalone` library module), overrides the per-user `%APPDATA%`
directory; the app hot-reloads it on a tightened poll (~150 ms) and `shot` reads it, plus explicit
`--presets <dir>` / `--preset-file <path>` flags that beat the env var. First user-visible
behavior: `LMV_PRESET_DIR=./presets cargo run -p standalone` and an edit to
`presets/fragment_aurora.toml` recolors the aurora on screen within ~150 ms.

## Context & problem

Preset TOML lives in `presets/`, is embedded into `core` at compile time, and is seeded
write-if-absent into `%APPDATA%\light-music-visualizer\presets` on first run. Both Rust frontends
read that per-user directory at runtime — the app loads it and hot-reloads on a 500 ms directory
poll, and Plan 0013's `shot` CLI loads it (else the embedded defaults).

So editing the **version-controlled** `presets/*.toml` reflects in neither the app nor `shot`
without a rebuild — and even then the seeded `%APPDATA%` copy shadows the embedded set. The user
wants to edit a file and see it immediately in both surfaces, against the repo folder that gets
committed.

The per-OS resolver is duplicated across `standalone/src/main.rs`, `standalone/examples/shot.rs`,
and `plugin-foobar/foo_lmv.cpp` (Plan 0007's close flagged that a rename silently un-shares the
copies). For "edit one file, see it in both," the app and `shot` **must** resolve the same
directory — drift breaks the feature — so consolidating the two Rust resolvers is part of the
work, not incidental cleanup.

## Decision

Per [ADR-0014](../adrs/0014-preset-dir-override-for-dev-iteration.md): add an **`LMV_PRESET_DIR`**
env var honored by both Rust frontends through a **shared resolver** extracted into a `standalone`
library module (bin + example resolve identically). An override loads and (for the app)
hot-reloads that directory and **skips seeding**. `shot` also gets **`--presets <dir>`** and
**`--preset-file <path>`** flags that win over the env var. Hot-reload stays the existing
**dependency-free poll**, tightened to ~150 ms — no filesystem watcher. We rejected a `--presets`
flag on the app (the winit app has no arg parser; an env var needs none), a `%APPDATA%`→repo
symlink (platform-fragile), the `notify` crate (dependency weight, NFR §4), keeping the resolver
duplicated (app/shot can drift), and moving resolution into `core` (host concern — source-agnostic
rule). Framed as a **power-user "custom preset folder"** knob, documented for end users as well as
the dev loop.

## Architecture diagram

```mermaid
flowchart TD
    subgraph lib["standalone/ (shared lib module - NEW)"]
        resolve["resolve_preset_dir()\nLMV_PRESET_DIR set? -> Override(dir)\nelse per-OS %APPDATA% -> Default(dir)\nelse Unresolved"]
    end

    subgraph app["standalone bin (main.rs)"]
        appstart["startup: resolve -> if Default: seed\nload_dir -> set_presets"]
        poll["poll ~150ms: dir_signature changed?\n-> reload_presets"]
    end

    subgraph shot["standalone example (shot.rs)"]
        shotres["--preset-file? -> one Preset\n--presets <dir>? -> load_dir\nelse resolve_preset_dir()\nelse embedded defaults"]
    end

    subgraph core["core/ (unchanged, source-agnostic)"]
        loaddir["preset::load_dir / from_toml_str"]
        setp["Renderer::set_presets / capture_preset"]
    end

    resolve --> appstart --> loaddir
    resolve --> shotres --> loaddir
    appstart --> setp
    poll --> loaddir
    shotres --> setp
    edit["edit presets/*.toml\n(version-controlled)"] -. LMV_PRESET_DIR=./presets .-> resolve
```

## Implementation phases

### Phase 1 — Shared resolver + `LMV_PRESET_DIR` override + live app reload
- **Owner skill:** dev
- **Area:** standalone
- **What:** Extract the per-OS preset-dir resolver into a shared `standalone` lib module, honor
  `LMV_PRESET_DIR`, rewire the app to use it (skip seeding when overridden), and tighten the poll.
- **Files touched:** new `standalone/src/lib.rs` (the resolver + `APP_DIR_NAME` + per-OS
  `preset_data_root` + a source-tagged `resolve_preset_dir`), `standalone/Cargo.toml` (add a
  `[lib]` target alongside the `[[bin]]`), `standalone/src/main.rs` (call the lib; seed only on the
  default path; `PRESET_POLL` 500 ms → ~150 ms).
- **Details:** `resolve_preset_dir()` returns the directory **and** whether it came from the
  override, so `main.rs` seeds only when it did not. `LMV_PRESET_DIR` set to a non-empty path wins;
  otherwise the existing per-OS `%APPDATA%`/`HOME`/`XDG` resolution stands; an empty/absent result
  degrades to embedded defaults exactly as today (NFR §10). `resolve_log_path()` keeps using the
  per-OS data root (the diagnostics log is unaffected by the override).
- **Done when:** a `standalone` lib unit test asserts `resolve_preset_dir()` returns the
  `LMV_PRESET_DIR` path tagged as overridden when the env var is set, and the per-OS default
  (tagged not-overridden) when it is unset. `cargo build -p standalone` builds bin + lib +
  example. (On-device, per the repo's visual-done-when posture: launching
  `LMV_PRESET_DIR=./presets cargo run -p standalone` and editing `presets/fragment_aurora.toml`
  recolors the aurora within ~150 ms — a human confirmation, see Risks.)

### Phase 2 — `shot` `--presets` / `--preset-file` flags on the shared resolver
- **Owner skill:** dev
- **Area:** standalone
- **What:** Give `shot` explicit directory/file overrides and route its library loading through
  the shared resolver, deleting its duplicated per-OS copy.
- **Files touched:** `standalone/examples/shot.rs` (use `standalone::…` resolver; add `--presets`
  / `--preset-file`; precedence).
- **Details:** Precedence, highest first: `--preset-file <path>` (one `Preset::from_toml_str` over
  the file's contents → a one-entry roster) → `--presets <dir>` (`load_dir`) → the shared
  `resolve_preset_dir()` (which honors `LMV_PRESET_DIR`, else `%APPDATA%`) → embedded defaults.
  The printed `[source]` label reflects which won. Bad `--presets`/`--preset-file` paths (missing
  dir with no valid presets, unreadable/malformed file) exit non-zero with a message.
- **Done when:** `cargo run -p standalone --example shot -- --presets presets --preset Aurora --out
  a.png` renders the **repo** `presets/` copy (not `%APPDATA%`); `--preset-file presets/<f>.toml`
  renders that single file; editing the file changes the next shot's PNG; `LMV_PRESET_DIR=./presets
  … --report` reports on the repo library; a bad `--presets`/`--preset-file` exits non-zero.

### Phase 3 — Document the loop + consolidate the shared-path note
- **Owner skill:** dev
- **Area:** docs
- **What:** Document the dev iteration loop and the power-user knob, and record that the Rust
  resolver is now single-sourced.
- **Files touched:** `docs/capturing.md` (the `LMV_PRESET_DIR=./presets` edit→live loop for app +
  `shot`; the new flags), `docs/presets.md` (the `LMV_PRESET_DIR` "custom preset folder" override),
  `README.md` (one line under the presets/visual-QA sections), a cross-referencing comment in
  `standalone/src/lib.rs` and `plugin-foobar/foo_lmv.cpp` noting the Rust side now shares one
  resolver while the C++ plugin resolves the same `%APPDATA%` dir independently (closes Plan 0007's
  duplicated-path minor for Rust).
- **Done when:** `docs/capturing.md`, `docs/presets.md`, and `README.md` describe `LMV_PRESET_DIR`
  and the `shot` flags with runnable commands; the C++/Rust resolvers carry the cross-reference.

## Data shapes

```rust
// illustrative — not the final interface. Lives in standalone/src/lib.rs.

/// Where the preset directory came from, so the app knows whether to seed.
pub enum PresetDir {
    /// LMV_PRESET_DIR pointed here — user-owned; do not seed.
    Override(std::path::PathBuf),
    /// The per-OS %APPDATA%/HOME/XDG default — seed write-if-absent on first run.
    Default(std::path::PathBuf),
    /// No data root could be resolved — caller keeps embedded defaults (NFR §10).
    Unresolved,
}

/// LMV_PRESET_DIR wins; else the existing per-OS resolution; else Unresolved.
pub fn resolve_preset_dir() -> PresetDir;
```

`shot`'s precedence chain (highest first): `--preset-file` → `--presets` → `resolve_preset_dir()`
(env-aware) → `default_presets()`.

## Risks & open questions

- **Live-app hot-reload is an on-device visual check.** The mechanism is unit-testable (resolver
  test) and the poll interval is code-verifiable, but "the aurora recolors within ~150 ms in the
  running window" is a GPU/on-device judgment, consistent with the visual done-whens carried
  forward in Plans 0003/0008. Not a blocker to closing.
- **Poll cadence vs CPU.** ~150 ms polling reads a directory listing 6-7×/s while the window is
  focused; `dir_signature` is a cheap `read_dir` + mtime scan, negligible next to rendering. If it
  ever matters, the interval is one constant.
- **Override dir mid-edit / partial writes.** An editor writing a file may momentarily produce a
  malformed TOML; `load_dir` already reports and skips bad files and `reload_presets` keeps the
  last good set (NFR §10), so a mid-save poll can't crash or blank the app.
- **`LMV_PRESET_DIR` pointing at a non-existent or empty dir.** Resolver returns `Override` with
  that path; `load_dir` yields nothing; both surfaces degrade to embedded defaults — same as an
  empty `%APPDATA%`. Documented, not an error.
- **Standalone gains a lib target.** Minor build-graph change; the bin links the lib, so the
  shipped binary's code is unchanged (no size impact, no new dependency). Consistent with ADR-0014.

## What this plan does NOT do

- **No foobar2000 plugin support.** The C++ plugin resolves its directory independently and keeps
  reading the shared `%APPDATA%` dir; honoring `LMV_PRESET_DIR` there is a followup, not this plan.
- **No filesystem watcher / `notify` dependency** — polling only (ADR-0014 C).
- **No app CLI argument parser** — the app takes the override via env var only (ADR-0014 A).
- **No change to seeding of the default `%APPDATA%` dir**, the embedded-preset compile-time
  embedding, or the C ABI (untouched, still v3).
- **No preset *authoring* changes** — the TOML schema, systems, and expression language are
  ADR-0002 / Plan 0007 territory and unchanged here.

## Followups (after this lands)

- Honor `LMV_PRESET_DIR` in the foobar2000 plugin's C++ resolver for cross-frontend parity.
- If a truly instant reload is ever wanted for a demo, revisit a watcher behind a dev-only feature
  flag (still keeping it out of the default shipped build).
