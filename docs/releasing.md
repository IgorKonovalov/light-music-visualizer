# Releasing

How the application version moves. The scheme is decided in
[ADR-0005](adrs/0005-versioning-and-release-cadence.md); this note is the operational
summary.

## One version, one command, once per plan

- **Single source of truth:** root `Cargo.toml` `[workspace.package].version`. Both crates
  inherit it (`version.workspace = true`); nothing else holds an app-version string.
- **Bump authority:** [`cargo-release`](https://github.com/crate-ci/cargo-release), a dev
  tool installed with `cargo install cargo-release` (not a workspace dependency). Config is
  in `release.toml`.
- **Cadence & owner:** one bump per shipped plan, run by the **architect in the close
  ceremony** — after the plan flips to `done` and its docs land. Not per phase commit.
- **No push:** `cargo-release` stages the version edit and writes the `vX.Y.Z` tag but does
  not push; the user pushes (project no-auto-push rule).

## Commands

```sh
# Preview (does nothing):
cargo release <patch|minor> --no-push --dry-run

# Do it (bumps the workspace version, commits, tags vX.Y.Z, no push):
cargo release <patch|minor> --no-push
```

While in the `0.x` band: a feature-plan is a **minor** bump (`0.1.0 -> 0.2.0`), a fix-only
plan is a **patch** bump (`0.1.0 -> 0.1.1`), and a docs/chore-only plan legitimately gets
**no** bump (choose the level deliberately — this is not a missed step). Reaching `1.0.0` is
a deliberate future act (freezing the C ABI and standalone behavior), never backed into.

## What this does NOT touch

- **The C ABI version** (`LMV_ABI_VERSION`, `core/src/ffi.rs`) is a **separate axis**
  (ADR-0003). It moves only when the `extern "C"` surface changes shape — never on an app
  bump, and an ABI bump never implies an app bump.
- **Dependency versions** (exact `=` pins, cargo-deny) are unrelated.
- **The foobar plugin** ships as a `.fb2k-component` with its own independent version;
  `cargo-release` does not drive it (ADR-0005).

## Where the version surfaces

- The standalone window title (`env!("CARGO_PKG_VERSION")`, resolves to the workspace
  version).
- The `vX.Y.Z` git tag and the GitHub release-zip name (NFR section 8).
