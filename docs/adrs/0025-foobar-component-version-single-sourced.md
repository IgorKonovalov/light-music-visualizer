# ADR-0025 — Single-source the foobar component version from the workspace version (generated header)

> **Status:** proposed
> **Date:** 2026-07-23
> **Related plan(s):** [0024-foobar-component-version-single-source](../plans/0024-foobar-component-version-single-source.md); supplements [ADR-0005](0005-versioning-and-release-cadence.md) (app versioning / one workspace version); revises the "plugin version remains independent" note from [Plan 0006](../plans/done/0006-versioning-wiring.md)

## Context

The foobar2000 component advertises its version through `DECLARE_COMPONENT_VERSION` in
`plugin-foobar/foo_lmv.cpp` — a **hardcoded string literal**, `"0.1.0"`, untouched since Plan 0001.
The application, meanwhile, is at `0.7.0` (root `Cargo.toml` `[workspace.package].version`, bumped
by `cargo-release` once per plan at the architect close per ADR-0005). foobar's Components list shows
`0.1.0`, which now misrepresents an app that has shipped six plans of plugin-affecting change.

Plan 0006's close note recorded that "the foobar plugin version remains independent." In practice that
independence was never exercised — the literal simply went stale, because nothing bumps it and no
cadence was ever defined for it. The project tracks three version axes: the **app** (workspace
version), the **C ABI** (`LMV_ABI_VERSION`, moves only on an `extern "C"` shape change, ADR-0003), and
this **component** version. The middle axis has a real, independent reason to differ; the component
axis does not — a user reading "Light Music Visualizer 0.x" wants to know *which app build* the plugin
corresponds to.

The decision is therefore twofold: (1) make the component version **track the workspace version**
rather than float independently, and (2) pick a mechanism, given that `DECLARE_COMPONENT_VERSION`
requires a compile-time **string literal** and the plugin builds **only** through
`plugin-foobar/build.ps1` (MSVC), never `cargo`.

## Decision

We will make the foobar component version **equal the workspace `[workspace.package].version`**, fed
in at build time by a **generated header**. `plugin-foobar/build.ps1` reads the version from root
`Cargo.toml` and writes `plugin-foobar/build/foo_lmv_version.h` containing
`#define FOO_LMV_VERSION "X.Y.Z"`; `foo_lmv.cpp` includes it and passes `FOO_LMV_VERSION` to
`DECLARE_COMPONENT_VERSION`, with an `#if __has_include` guard plus an `#ifndef FOO_LMV_VERSION`
fallback (`"0.0.0-dev"`) so a compile outside `build.ps1` (editor tooling, a stray direct `cl`) still
builds. The generated header lands in `plugin-foobar/build/`, which is already gitignored — no new
tracked file. This **revises** Plan 0006's independent-plugin-version note: the component version is no
longer an independent axis. The **C ABI version stays fully independent** — this decision does not
touch `LMV_ABI_VERSION`.

## Consequences

### Positive
- **The version never drifts again.** It is derived from the one workspace source of truth, so every
  `cargo-release` bump at a plan close flows into the plugin with zero extra steps.
- **The number becomes meaningful to users** — it names the app build the plugin matches, instead of a
  frozen `0.1.0`.
- **No quote-escaping fragility.** A generated `#define` sidesteps passing a quoted string literal
  through `build.ps1`'s nested `cmd /c "\"$vcvars\" >nul && $cl"` wrapper.
- **No new tracked artifact.** The header lives in the already-ignored `build/` dir.

### Negative (the price we pay)
- **The plugin loses its independent bump lever.** A plugin-only fix that lands *between* plan closes
  (e.g. the mid-playback render fix, commit `88f9769`) does not move the number until the next
  workspace bump. Accepted: under the current cadence the workspace version is the meaningful unit, and
  a plugin-only SemVer nobody maintained is worse than one that tracks the app.
- **A compile outside `build.ps1` shows `0.0.0-dev`.** Correct-by-design (that path has no version
  source), and `build.ps1` is the only supported plugin build.
- **`build.ps1` now parses `Cargo.toml`.** A trivial regex dependency on the file's shape; it must
  anchor to the `[workspace.package]` section so it can never pick up a member-crate version.

### Neutral
- **C ABI untouched** (`LMV_ABI_VERSION` unchanged) — the three axes remain three; only the component
  axis is redefined as a mirror of the app axis.
- No core or Rust change; this is a build-script + C++ shim + docs matter.

## Alternatives considered

### Alternative A — `build.ps1` injects a `/DFOO_LMV_VERSION="X.Y.Z"` cl define
No generated file; the version lives only as a compile define. Rejected: producing a **quoted string
literal** define and threading it intact through `build.ps1`'s `cmd /c "\"$vcvars\" >nul && $cl"`
compile line is brittle quote-escaping (PowerShell → cmd → cl), and a bare-token define can't form a
string literal without an extra stringizing macro. The generated header is strictly more robust for the
same result.

### Alternative B — A committed `version.h` with the literal
Dead simple, no build logic. Rejected: it is just the current hardcoded literal relocated to another
hand-maintained file — **not single-sourced**, so it drifts exactly as `0.1.0` did. Defeats the goal.

### Alternative C — Leave the component version independent, bump it by hand
Keep Plan 0006's stance and define a manual cadence. Rejected: six plans of silence show the manual
lever is never pulled, and the component axis has no genuine reason to differ from the app the way the
C ABI axis does.

## Notes
- Pairs with the stale-description refresh folded into Plan 0024 (the `DECLARE_COMPONENT_VERSION` and
  `ui_element` descriptions still name the culled spectrum/pulse/starfield scenes) — content only, no
  bearing on this versioning decision.
- The workspace-version **bump cadence** (cargo-release at plan close, ADR-0005) is unchanged; this ADR
  only decides how the *component* version is derived from it.
