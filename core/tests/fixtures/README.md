# Golden drift fixtures

These TOML files are **test-only frozen fixtures** for the golden drift guard
(`core/tests/golden.rs`), one per `SystemKind`. They exist to catch **unintended
engine rendering drift** — a shader or scene-math change that silently perturbs
output — by pinning each scene's pixels to a committed baseline PNG under
`core/tests/golden/`.

Decision and rationale: [ADR-0023](../../../docs/adrs/0023-golden-drift-guard-uses-frozen-fixtures.md)
(Plan 0022).

## Do not tune

**These are not shipped presets and must not be tuned for looks.** Editing a
fixture changes its render and invalidates the committed baseline, defeating the
drift guard. The shipped presets in `presets/` are the ones the `preset-author`
lane tunes; they are guarded *behaviorally* elsewhere (`sanity`, `reactivity`,
`animation` — all iterate `default_presets()`), never pixel-pinned here. Each
fixture is deliberately minimal and deterministic (constant or lightly-bound
params) so it draws a non-trivial frame that never needs content tuning.

## Adding a scene

A new `SystemKind` variant makes `golden.rs` fail to compile until you add its
fixture here — the fixture roster is an **exhaustive `match SystemKind`** with no
wildcard arm. To add one:

1. Author `<system_name>.toml` here (mirror the header comment of the others).
2. Add the variant's arm to `fixture()` in `golden.rs`, and the variant to the
   `SYSTEMS` list.
3. Bless the baseline on Windows WARP:
   `LMV_BLESS=1 cargo test -p lmv-core --test golden`, then eyeball the new PNG
   under `core/tests/golden/` to confirm the scene actually drew.

Baselines are WARP-only (macOS skips per ADR-0016) and must be blessed on WARP or
they will drift.
