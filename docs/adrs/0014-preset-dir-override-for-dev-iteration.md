# ADR-0014 — Preset-directory override (`LMV_PRESET_DIR`) with a shared resolver, polling over a watcher

> **Status:** proposed
> **Date:** 2026-07-22
> **Related plan(s):** [0015](../plans/0015-preset-dir-override-and-live-iteration.md)
> **Related:** [ADR-0002](0002-layered-preset-architecture.md) (the preset layer being edited),
> [Plan 0007](../plans/done/0007-curated-preset-library.md) (seed-to-`%APPDATA%` + the duplicated
> per-OS resolver this consolidates), [Plan 0013](../plans/done/0013-headless-scene-capture.md)
> (the `shot` CLI that added a third copy of the resolver)

## Context

Preset TOML lives in the repo under `presets/`, is embedded into the core at compile time
(`include_str!` → `default_presets()`), and is seeded write-if-absent into a per-user directory
on first run (`%APPDATA%\light-music-visualizer\presets` on Windows). At runtime both Rust
frontends read that per-user directory: the standalone app loads it and **hot-reloads** on a
500 ms directory poll (newest-mtime + file-count signature → `set_presets`), and the Plan 0013
`shot` capture CLI loads it (else the embedded defaults).

That makes iterating on a preset awkward. Editing a **version-controlled** `presets/*.toml` file
reflects in neither the app nor `shot` without a rebuild — and even after a rebuild the seeded
`%APPDATA%` copy shadows the embedded set. A developer or agent wants to **edit one file and see
the change immediately** in both the running app and a headless `shot`.

Two forces shape the fix:

- **The app and `shot` must resolve the *same* directory.** The whole point — "edit one file,
  see it in both" — collapses the instant their resolvers disagree. The per-OS resolver is
  currently duplicated across `standalone/src/main.rs`, `standalone/examples/shot.rs`, and (in
  C++) `plugin-foobar/foo_lmv.cpp`; Plan 0007's close already flagged that a rename silently
  un-shares the copies.
- **"Lightweight is a feature" (CLAUDE.md / NFR §4).** Any new mechanism should avoid adding a
  dependency to the shipped binary.

## Decision

We will add an **`LMV_PRESET_DIR`** environment variable honored by **both Rust frontends**: when
it names a directory, that directory overrides the resolved preset directory — the app and `shot`
both load and (for the app) hot-reload it. `shot` additionally accepts explicit **`--presets
<dir>`** and **`--preset-file <path>`** flags, which win over the env var. The per-OS resolver is
**extracted into a shared `standalone` library module** so the binary and the example resolve
identically, removing the Rust-side duplication and guaranteeing the two agree. Hot-reload stays
the existing **dependency-free directory poll**, with the interval tightened (~500 ms → ~150 ms)
so edits feel immediate; we do **not** add a filesystem watcher.

## Consequences

### Positive
- Editing a version-controlled `presets/*.toml` shows up live in the running app within ~150 ms
  and is picked up by the next `shot` — a real iteration loop with no rebuild and no relaunch.
- One shared resolver: the app and `shot` cannot drift, resolving Plan 0007's duplicated-path
  minor on the Rust side.
- A general power-user affordance — "point me at a custom preset folder" — that is zero-cost when
  the variable is unset.
- No new dependency in the shipped `lmv.exe`; with `LMV_PRESET_DIR` unset the app behaves exactly
  as before.

### Negative
- Reload is ≤~150 ms polled, not sub-frame. A watcher would be instant but costs a dependency —
  rejected below.
- The `standalone` crate gains a **library target** alongside its binary so the example can share
  code. This is a small structural change; it *clarifies* rather than contradicts Plan 0013's
  rejection of "a standalone `src/lib.rs`" — that rejection was about where capture **tests** live
  (answer: in `core`, still true), whereas this lib hosts only host-utility path resolution, no
  tests.
- An override **skips seeding** (the override directory is user-owned, not ours to populate). An
  empty or all-malformed override directory degrades to the embedded defaults — the same
  never-crash behavior as today (NFR §10).

### Neutral
- The foobar2000 plugin (C++) resolves its directory independently and is **out of scope**; it
  keeps reading the shared `%APPDATA%` directory. Honoring `LMV_PRESET_DIR` there is a possible
  followup, not part of this decision.

## Alternatives considered

### A — A `--presets` CLI flag on the standalone app instead of an env var
Rejected. The `winit` app parses no arguments today; an env var needs no parser (the app already
reads env vars for the data root), and a single `set LMV_PRESET_DIR=…` points *both* the app and
`shot` at the repo folder for a whole dev session. `shot` still gets flags because it already
parses arguments and one-off / agent captures want them explicit.

### B — A symlink from `%APPDATA%\…\presets` to the repo `presets/`
Rejected. Zero-code, but platform-fragile (Windows symlinks need admin / developer mode),
implicit and invisible, and it does nothing for pointing `shot` at an arbitrary folder.

### C — A filesystem watcher (`notify` crate) for sub-frame reload
Rejected. It pulls several transitive crates into the shipped app, against "lightweight is a
feature" (NFR §4). The existing poll already reloads on any edit, and ~150 ms is imperceptible
while editing — the win doesn't justify the weight.

### D — Keep the resolver duplicated in `main.rs` and `shot.rs`, adding the env override to both
Rejected. The feature's core invariant is that the app and `shot` resolve the **same** directory;
two copies can drift (Plan 0007 already flagged exactly this), silently breaking "edit one file,
see it in both."

### E — Move the resolver into `core`
Rejected. `%APPDATA%` / `HOME` / `XDG` conventions are a host concern. The core stays
source-agnostic and platform-free (CLAUDE.md); directory resolution belongs in the shell.

## Notes

Pairs with Plan 0015, which owns the phasing and the exact resolver/flag shapes. This ADR is
accepted at that plan's close per the project workflow.
