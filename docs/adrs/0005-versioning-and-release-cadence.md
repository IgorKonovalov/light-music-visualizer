# ADR-0005 — App versioning: SemVer 0.x, one workspace-inherited version, cargo-release as the single bump authority run at plan close

> **Status:** proposed
> **Date:** 2026-07-21
> **Related plan(s):** 0006-versioning-wiring (accepted at that plan's close)

## Context

The workspace carries **two** version strings today — `lmv-core` `0.1.0` and `standalone`
`0.1.0` (`core/Cargo.toml:5`, `standalone/Cargo.toml:4`) — and neither has moved across
Plans 0001-0005. The root `[workspace.package]` inherits `edition`/`license`/`repository`
but has **no** `version`. As with the sibling repo before its ADR-0087, nothing owns the
bump: there is no single source of truth, no tooling, and no cadence, so the number will
drift into a lie (a packaged release zip or `git tag` reading `0.1.0` forever) or the two
strings will disagree.

Three axes must not be conflated:

1. **The application version** — the human-facing "how much has shipped" number, surfaced
   in the standalone window title (`standalone/src/main.rs:121`, `:204`, `:253`), the future
   plugin about box, and the NFR §8 release-zip / `git tag` name. This is what this ADR
   governs.
2. **The C ABI contract version** — `LMV_ABI_VERSION = 1` (`core/src/ffi.rs:40`), the
   versioned shape the C++ plugin links against (ADR-0003). This moves **only** when the
   `extern "C"` surface changes shape, which is itself an ADR-worthy event. It is entirely
   independent of the app version: the app can ship many minor versions while the ABI stays
   at `1`.
3. **Dependency versions** — exact-pinned per crate (`=` in `Cargo.toml`), governed by the
   lightweight-is-a-feature rule and cargo-deny. Unrelated to this ADR.

This repo differs from `market-analyzer` in ways that make translation the real design work:
it is a **Cargo workspace, not Python**; the follower is a **C++ `.fb2k-component`, not a
`package.json`**; and "lightweight is a feature" argues against pulling a Python toolchain
(commitizen) into a pure-Rust repo.

## Decision

**Semantic versioning, held in the `0.x` band, with a single workspace-inherited version
string.** We add `version = "0.1.0"` to root `[workspace.package]` and change both member
crates to `version.workspace = true`, so there is exactly **one** canonical app-version
string for the whole workspace. We keep the honest starting number **`0.1.0`** — this
project is genuinely young (Plan 0006), so "early, unreleased" is true; we are not inflating
to signal maturity we haven't accumulated.

**`cargo-release` is the single bump authority.** It is Rust-native (no Python runtime),
understands workspace-inherited versions, and can bump the canonical string and create a
`vX.Y.Z` tag without pushing. Its config lives in `release.toml`. While in `0.x`, a `minor`
bump (`0.1.0 → 0.2.0`) marks a feature-plan and a `patch` bump (`0.1.0 → 0.1.1`) marks a
fix-only plan; reaching `1.0.0` is a deliberate future act (freezing the C ABI and the
standalone's public behavior), never a number we back into. `cargo-release` is a
`cargo install` dev tool, not a workspace dependency, so it adds nothing to the shipped
binary or the dependency-audit surface.

**The bump runs once per plan, in the architect's close ceremony**, right after the plan
flips to `done` and its docs land — one version bump per shipped plan (the unit a human
reads as "a feature"), not one per phase commit. `cargo-release` stages the version edit and
writes the `vX.Y.Z` tag but **does not push**, consistent with the project's no-auto-push
rule; the user pushes.

**The C ABI version stays a separate axis.** `LMV_ABI_VERSION` is governed by ADR-0003 and
moves only on an ABI shape change; a `cargo-release` app bump never touches it, and an ABI
bump never implies an app bump. The two are decoupled on purpose.

**The foobar plugin's component version is independent, not a synced follower.** The plugin
is unbuilt C++ that will ship as a `.fb2k-component` with its own version notion; what
actually couples it to the core is the **C ABI version it links against**, not the app
version. When the plugin is built, its about box should surface the core app version it was
built from (via the ABI or a build constant) for support/debugging, but its own component
version is set by the plugin, not driven by `cargo-release`. This is revisited if and when
the plugin ships (a follow-up plan under roadmap item 5, packaging & release).

## Consequences

### Positive
- One canonical version string, one command to move it, no two-file drift — the app version
  becomes truthful and stays truthful.
- Staying in `0.x` with a deliberate `1.0.0` removes the "accidental 1.0" trap and honestly
  signals a pre-release app.
- "One bump per plan" gives the minor number a human-readable meaning: roughly how many
  feature-plans have shipped since 0.1.0.
- Fits the existing ceremony exactly — architect already owns the close, commits docs by
  explicit path, and refrains from pushing. The bump is one more close step, not a new
  workflow. No new runtime dependency and no Python toolchain.

### Negative
- **The bump is a manual close step that can be forgotten** — there is no CI gate forcing
  one bump per plan. Mitigated by naming the owner (architect) and the moment (close) and
  adding it to the close checklist; discipline still carries it. We accept this over a
  per-commit CI auto-bump, which would inflate the minor several times per plan and couple
  version state to push timing (contradicting no-auto-push).
- **`cargo-release` writes a git tag**, so a mistaken bump leaves a tag behind — a sharper
  edge than a plain edit. Tags are cheap to delete locally before push; since we never
  rewrite history, a bad *pushed* tag is corrected forward, not amended.
- **A plan that ships only `chore`/`docs`/`refactor` work may warrant no bump.** Unlike a
  conventional-commit-driven tool, `cargo-release` takes the increment we pass, so the closer
  must choose `patch`/`minor`/none deliberately — a docs-only plan legitimately gets no bump,
  which must not be mistaken for a missed step.

### Neutral
- The foobar plugin version being independent means the plugin and the standalone need not
  share a number — correct, since they are separate artifacts with separate release notions,
  unlike a single app whose renderer and sidecar ship together.
- Moving `version` into `[workspace.package]` is a one-time, behavior-neutral refactor;
  `0.1.0` is unchanged, so no tag history is invalidated (there are no tags yet).

## Alternatives considered

### Alternative A — Per-crate independent versions
Keep each crate's `[package].version` and bump them separately. Rejected: while `lmv-core`
and `standalone` ship together as one app, two strings is two things to keep in sync and a
heavier `cargo-release` config for no benefit. A single workspace-inherited string is the
Cargo idiom and gives the bump tool exactly one line to edit. (If the crates ever ship on
independent cadences, this ADR is superseded.)

### Alternative B — commitizen with its cargo provider
Mirror `market-analyzer`'s bump authority exactly (single tool, `major_version_zero`,
version-file sync). Rejected on "lightweight is a feature": commitizen pulls a **Python
toolchain** into a pure-Rust workspace for a job a Rust-native tool does. The conventional-
commit-driven auto-increment is a real convenience we give up, but choosing the increment by
hand once per plan (patch/minor) is trivial and avoids the cross-ecosystem dependency.

### Alternative C — cargo-smart-release
Also Rust-native. Rejected as more machinery than the cadence needs: it is oriented toward
crates.io publishing with dependency-graph-aware release ordering, and neither `lmv-core` nor
`standalone` is published to crates.io (`publish = false`). `cargo-release` covers the
"bump the workspace version + tag, don't push" job with less surface.

### Alternative D — Start at 0.5.0 ("mature but unreleased")
`market-analyzer` chose this to signal ~90 plans of accumulated substance below 1.0. Rejected
here on the user's call: this project is at Plan 0006, so `0.1.0` is the honest number.
`0.5.0` would overstate maturity. Both are pre-1.0 and mechanically identical going forward;
the only difference is the starting integer, and here the truthful one is `0.1.0`.

### Alternative E — Do nothing / bump the two strings by hand
The status quo. Rejected: it is exactly how the version stalled at `0.1.0` across five plans.
Two hand-edited strings with no owner and no tag is the drift this ADR ends.

## Notes
- Mechanics after this ADR (wired by Plan 0006): `cargo release <patch|minor> --no-push` at
  plan close; `cargo release <level> --no-push --dry-run` to preview; the current version is
  read from root `[workspace.package].version`. Tag format `vX.Y.Z`.
- This ADR governs the *application* version only. The C ABI contract version
  (`LMV_ABI_VERSION`, ADR-0003) and dependency pins (exact `=`, cargo-deny) are separate axes
  and unaffected.
- The architect close-ceremony checklist gains a "choose level, `cargo release --no-push`,
  confirm the tag" step (Plan 0006 wires the tooling and documents it).
