# ADR-0022 — Build-time embedding of the preset library (generated, not hand-maintained)

> **Status:** proposed
> **Date:** 2026-07-23
> **Related plan(s):** [0021](../plans/0021-decouple-preset-content-from-code.md)

## Context

`core` embeds the shipped presets at compile time so the C-ABI / foobar path and `default_presets()`
render without any preset directory (ADR-0006, Plan 0007). Today that embedding is a **hand-maintained
list**: `core/src/preset/mod.rs` holds `const EMBEDDED: [(&str, &str); 18]`, one
`include_str!("../../../presets/<file>")` tuple per preset. Adding a preset — pure **content** — forces
three code edits across two files: the tuple, the array length (`; 18`), and a hardcoded count assert
in `core/tests/preset.rs` (`assert_eq!(presets.len(), 18, …)`). The `preset-author` lane documents
this as a required ritual (its `api-feedback.md` curation handoff), which is the smell: shipping a
`.toml` should not touch Rust.

The embedding requirement is real and stays — the constraint is purely mechanical: `include_str!`
accepts only **string-literal** paths, so it cannot expand a directory, which is why the list is
written by hand. Rust's standard escape hatch for "embed a directory whose contents vary" is a build
script that generates the list at build time. The project has **no build script yet**, and it is
deliberately dependency-frugal (every crate is a justified cost; direct deps are exact-pinned), so the
choice of mechanism — a zero-dependency build script vs. a macro crate — is a real decision.

## Decision

We will add `core/build.rs` (zero-dependency) that scans `../presets/*.toml` at build time, sorts the
filenames for a deterministic order, and generates `$OUT_DIR/embedded_presets.rs` containing the
`EMBEDDED` slice — each entry a `(filename, include_str!(<absolute path>))` tuple, so rustc still
embeds the bytes exactly as today. `core/src/preset/mod.rs` replaces its hand-written array with
`include!(concat!(env!("OUT_DIR"), "/embedded_presets.rs"))` and keeps `default_presets`/`seed_dir`
unchanged (they already iterate `EMBEDDED`). The build script emits
`cargo:rerun-if-changed=../presets` so adding, removing, or editing a preset retriggers generation.
The count assert becomes structural — "every embedded preset parses" plus a floor (`>= 8`) — never an
exact number. Net effect: **drop a `.toml` in `presets/`, rebuild, and it ships** with no Rust edit and
no count to bump.

## Consequences

### Positive
- **Content is decoupled from code.** The `presets/` directory is the single source of truth; the
  embedded set is derived from it, so it cannot drift and there is nothing to hand-bump.
- **Simplifies the `preset-author → dev` curation handoff** (ADR-0017): embedding a strong preset stops
  being a coupled two-file Rust edit and becomes "commit the `.toml` to `presets/`." That skill's
  handoff note should be updated to match (a followup).
- **No new dependency** — a zero-dep build script fits the lightweight/frugal ethos.
- The generated order is sorted and deterministic, matching the on-disk `load_dir` cycle order.

### Negative (the price we pay)
- **The project's first build script.** Build scripts run on every build, must be kept simple and
  fast (a directory glob is instant), and are executed by CI and rust-analyzer — a new build-time
  surface to keep honest. Zero-dep and ~30 lines keeps the cost low.
- **The embedded list is now `include!`-generated**, so it is less greppable than a literal array — a
  reader running `grep EMBEDDED` finds the `include!` and the build script, not the entries. Mitigated
  with a pointer comment in `mod.rs` and `build.rs`.
- **The `presets/` path becomes a build-time dependency of `core`.** Relocating or renaming the
  repo-root `presets/` dir now breaks the build (it already did, implicitly, via `include_str!`); the
  path stays a fixed convention.

### Neutral
- The build re-runs when `presets/` changes — desired behavior, and the reason the coupling disappears.

## Alternatives considered

### Alternative A — the `include_dir` crate
A proc-macro crate that embeds a directory tree (`include_dir!("$CARGO_MANIFEST_DIR/../presets")`) and
iterates entries at runtime. Cleaner call site than a build script, but it is a **new dependency** —
ADR-worthy by our dependency rule, with a proc-macro build cost, and it embeds every file (needing a
`.toml` filter). Rejected: a zero-dep build script gets the same decoupling without adding a crate.

### Alternative B — status quo, or a slice-only tweak
Keep the hand-written list, optionally switching `[(&str,&str); 18]` to `&[(&str,&str)]` so the length
literal and the exact-count assert disappear. Removes two of the three edits, but the per-file
`include_str!` tuple is still hand-written, so content stays coupled to code. Rejected as a half-fix.

### Alternative C — drop embedding, load from disk only
Remove `EMBEDDED` and always `load_dir`. Eliminates the coupling entirely but breaks the guarantee that
the C-ABI / foobar path renders without a preset directory (ADR-0006). Non-starter.

### Alternative D — an in-repo proc-macro
Write our own `include_dir!`-style proc-macro in a workspace crate. More machinery (a proc-macro crate,
its own compile) than a build script for an identical result. Rejected.

## Notes

Motivated by a user coupling report (2026-07-23) against `core/src/preset/mod.rs`. Plan 0021 also folds
in a related DRY cleanup — centralizing the `SystemKind` name↔kind mapping, duplicated across
`schema.rs::from_name`, `shot.rs::parse_system`, and `shot.rs::system_name` — but that is a
straightforward single-source refactor with no rejected alternative, so it lives in the plan, not here.
