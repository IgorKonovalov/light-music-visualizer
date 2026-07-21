# 0006 — Versioning: single source of truth + cargo-release + surfacing

> **Status:** done
> **Created:** 2026-07-21
> **Closed:** 2026-07-21
> **Owner skill(s):** dev, human
> **Related ADRs:** [0005](../adrs/0005-versioning-and-release-cadence.md) (accepted at this close)

**Close summary (Mode 4, fresh session).** Phases 1-3 (`dev`) landed in commits ef5c4dd,
1298e2b, 3616dfb; Phase 4 (`human`) confirmed at close — `cargo-release 1.1.3` is installed
and the dry-run works. Review verdict: **clean, no blockers, no majors.** Verified live: both
crates resolve to `0.1.0` via workspace inheritance (`cargo metadata`); exactly one literal
app-version string (root `Cargo.toml`); the standalone title embeds `env!("CARGO_PKG_VERSION")`
at all three sites; `cargo release minor --no-push` dry-run edits the single workspace version
`0.1.0 -> 0.2.0` and creates a single `v0.2.0` tag via `shared-version`. The dry-run's
`Publishing ...` line was investigated and cleared — it is cargo-release's release-set summary
header, not a publish action; `publish = false` in both `release.toml` and each `Cargo.toml`
means no publish can occur. First bump run at this close: **minor, `0.1.0 -> 0.2.0`, tag
`v0.2.0`, no push** (feature-plan cadence per ADR-0005).

## TL;DR

Wire the versioning scheme decided in ADR-0005: collapse the workspace's two `0.1.0`
strings into one workspace-inherited version, adopt `cargo-release` as the single bump
authority (config committed, `--no-push`, tag `vX.Y.Z`), and surface the version in the
standalone window title. The result: one canonical app-version string, one command to move
it, run once per plan at the architect's close — starting honestly at `0.1.0`.

## Context & problem

The workspace carries two independent version strings (`lmv-core` and `standalone`, both
`0.1.0`) and root `[workspace.package]` has no `version`. Nothing owns the bump: no single
source of truth, no tooling, no cadence — so the number will drift into a lie or the two
strings will disagree. ADR-0005 chose the fix (workspace-inherited single version,
`cargo-release`, one bump per plan at close, keep `0.1.0`, C-ABI version stays a separate
axis, plugin version independent). This plan does the wiring.

## Decision

Implement ADR-0005 as chosen. We rejected per-crate versions (two strings to sync, no
benefit while the crates ship together), commitizen (pulls a Python toolchain into a pure-
Rust repo), cargo-smart-release (crates.io-publishing machinery we don't use, since both
crates are `publish = false`), and starting at 0.5.0 (overstates maturity at Plan 0006).

## Architecture diagram

```mermaid
flowchart TB
    subgraph axes["Three independent version axes (ADR-0005)"]
        APP["App version<br/>[workspace.package].version = 0.1.0<br/>(this plan; cargo-release bumps it)"]
        ABI["C ABI version<br/>LMV_ABI_VERSION = 1 (core/src/ffi.rs)<br/>(ADR-0003; moves only on ABI shape change)"]
        DEP["Dependency versions<br/>exact '=' pins + cargo-deny<br/>(unrelated)"]
    end
    APP -->|version.workspace = true| CORE["core/Cargo.toml"]
    APP -->|version.workspace = true| STD["standalone/Cargo.toml"]
    STD -->|env!\"CARGO_PKG_VERSION\"| TITLE["standalone window title"]
    APP -.->|cargo release <level> --no-push| TAG["git tag vX.Y.Z (architect close, no push)"]
    PLUG["foobar .fb2k-component<br/>independent version"] -.->|surfaces core version it links| ABI
```

## Implementation phases

Each phase ships as its own commit. `dev` runs all phases in one session; the architect
reviews the whole plan once at the end.

### Phase 1 — Single source of truth (workspace-inherited version)
- **Owner skill:** dev
- **Area:** core, standalone (Cargo manifests only)
- **What:** Add `version = "0.1.0"` to root `[workspace.package]`; change `core/Cargo.toml`
  and `standalone/Cargo.toml` from a literal `version = "0.1.0"` to `version.workspace = true`.
- **Files touched:** `Cargo.toml`, `core/Cargo.toml`, `standalone/Cargo.toml`.
- **Done when:** `cargo metadata --no-deps --format-version 1` shows both `lmv-core` and
  `standalone` at version `0.1.0` (now inherited from the workspace), and `cargo build`
  succeeds. There is exactly one literal version string in the workspace, in root
  `[workspace.package]`.

### Phase 2 — cargo-release config
- **Owner skill:** dev
- **Area:** repo root
- **What:** Add a committed `release.toml` configuring `cargo-release` for this workspace:
  operate on the single shared workspace version, tag format `vX.Y.Z` (`tag-prefix = "v"`),
  **never push** (`push = false`), and no crates.io publish (`publish = false`). Keep the
  config minimal — the tool is a `cargo install` dev tool, not a workspace dependency.
- **Files touched:** `release.toml`.
- **Done when:** with `cargo-release` installed, `cargo release patch --no-push --dry-run`
  (from the workspace root) previews a bump `0.1.0 → 0.1.1` on the single workspace version
  and a `v0.1.1` tag, with no push and no publish, and exits without error. (The dry-run is
  the acceptance check; no actual bump happens in this phase — the first real bump is the
  architect's close of this plan.)

### Phase 3 — Surface the version + document the release process
- **Owner skill:** dev
- **Area:** standalone, docs
- **What:** Include the version in the standalone window title via
  `env!("CARGO_PKG_VERSION")` (which now resolves to the workspace version), at the three
  title sites; and add a short "Releasing" note (in `CLAUDE.md`'s commit-hygiene area or a
  brief `docs/` note) stating the command, cadence (one bump per plan at close), owner
  (architect), and the `vX.Y.Z` / release-zip naming per NFR §8.
- **Files touched:** `standalone/src/main.rs` (title format at `:121`, `:204`, `:253`), and
  one of `CLAUDE.md` / a short `docs/` release note.
- **Done when:** `cargo run -p standalone` shows a window title containing the version (e.g.
  `light-music-visualizer 0.1.0 — <scene> — <fps> fps`), and the release process is written
  down where a future closer will find it. The plugin about box is **out of scope** (plugin
  unbuilt; ADR-0005 keeps its version independent).

### Phase 4 — Install tooling + confirm baseline
- **Owner skill:** human
- **What:** Install the bump authority locally (`cargo install cargo-release`) and run the
  Phase 2 dry-run to confirm it works in your environment; confirm `0.1.0` as the honest
  baseline. No push, no tag yet — the first real `vX.Y.Z` tag is created by the architect
  when this plan is closed (one bump per plan).
- **Done when:** `cargo release --version` succeeds locally and the dry-run previews the
  expected bump/tag; you have confirmed `0.1.0` is the starting number.

## Data shapes

No new runtime structs. The only "data" is manifest and config:

```toml
# illustrative — root Cargo.toml
[workspace.package]
version = "0.1.0"

# illustrative — release.toml (workspace root)
tag-prefix = "v"        # tags read vX.Y.Z
push = false            # never push (project no-auto-push rule)
publish = false         # crates are publish=false
```

## Risks & open questions

- **`cargo-release` on a `publish = false` workspace.** The tool is publish-oriented by
  default; the config must explicitly disable publish and push so a bump only edits the
  version and tags. The Phase 2 dry-run is the guard — if it tries to publish or push, the
  config is wrong.
- **`env!("CARGO_PKG_VERSION")` in `standalone`.** This resolves at compile time to the
  crate's version, which after Phase 1 is the inherited workspace version — verify the title
  actually shows `0.1.0`, not an empty/edition string.
- **Forgotten bump.** No CI gate forces the once-per-plan bump (ADR-0005 accepts this). The
  Phase 3 written process and the close-ceremony note are the mitigation; discipline carries
  it.

## What this plan does NOT do

- **Does not touch `LMV_ABI_VERSION`.** The C ABI contract version is a separate axis
  (ADR-0003, ADR-0005) and is not bumped by app releases.
- **Does not version the foobar plugin.** The plugin is unbuilt; ADR-0005 keeps its
  `.fb2k-component` version independent. Surfacing the core version in the plugin about box is
  a follow-up when the plugin ships (roadmap item 5, packaging & release).
- **Does not add a CI freshness/bump gate.** Cadence is manual-at-close by design.
- **Does not perform the first bump.** Wiring only; the first `cargo release` runs at this
  plan's architect close.

## Followups (after this lands)

- Architect close-ceremony checklist gains a "choose level, `cargo release <level> --no-push`,
  confirm the `vX.Y.Z` tag" step — added by the architect when accepting ADR-0005 / closing
  this plan (architect owns its own skill instructions).
- When the foobar plugin is built (roadmap item 5): surface the linked core version in its
  about box and decide the component-version relationship in that plan.
