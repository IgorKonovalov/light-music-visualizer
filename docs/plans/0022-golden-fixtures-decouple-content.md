# 0022 — Decouple the golden drift guard from shipped presets (per-system frozen fixtures)

> **Status:** draft
> **Created:** 2026-07-23
> **Owner skill(s):** dev
> **Related ADRs:** [0023](../adrs/0023-golden-drift-guard-uses-frozen-fixtures.md) (this plan's
> decision); [0016](../adrs/0016-gpu-tests-opt-in-ci-scope.md) (golden runs WARP-only, skips on a
> missing adapter); [0017](../adrs/0017-preset-author-skill-lane.md) (the content lane the coupling
> was hurting)

## TL;DR

`core/tests/golden.rs` pins its drift baselines to three **shipped, curated presets** (`Aurora`,
`Warp Drive`, `Drift`). Those are exactly what the `preset-author` lane tunes, so every intentional
content change trips the engine-drift alarm and reds CI (it did — commit `76a2fb4` moved all three
baselines to mean 0.15–0.25 vs a 0.02 tolerance). Repoint golden at **test-only frozen fixtures,
one per `SystemKind`**, authored as TOML under `core/tests/fixtures/` and keyed by an exhaustive
match so a new system must add a fixture. Drop all golden pixel-pinning of shipped presets (they
keep their behavioral floors in `sanity`/`reactivity`/`animation`) and delete the three shipped
baselines. Per [ADR-0023](../adrs/0023-golden-drift-guard-uses-frozen-fixtures.md). Test-only, no
production/C-ABI/`ci.yml` change.

## Context & problem

The golden test is meant to catch **unintended** rendering drift — a shader or scene-math change
that silently perturbs output. But it compares against baselines rendered from presets in the
shipped roster, looked up by name. The `preset-author` lane ([ADR-0017](../adrs/0017-preset-author-skill-lane.md))
exists to tune that roster, so an *intended* tune reads as an engine regression and reds CI until a
re-bless. Two things with opposite change cadences — stable engine rendering, churny content — are
welded together, and the seam crosses lanes (a content author reds a `dev`-owned test).

It also under-covers what it claims to guard: the current cases are two `fragment_field` presets and
one `swarm`. The three line-family systems (`parametric_curve`, `lsystem`, `star_pattern`) — which
feed a shared line renderer through **different** geometry generators — have **zero** golden
coverage, so a generator regression there is invisible.

The shipped presets are still guarded behaviorally: `sanity` (coverage + quadrant spread),
`reactivity` (per-band reaction floor), and `animation` (motion floor) all iterate
`default_presets()` and survive content tuning by design. Only the pixel-exact guard was pointed at
content it should not pin.

See [ADR-0023](../adrs/0023-golden-drift-guard-uses-frozen-fixtures.md) for the decision and the
rejected alternatives (keep golden on shipped presets; loose pin on shipped too; per-idiom
fixtures; embedded consts / reuse `default_presets()`).

## Decision

Repoint golden to render **one frozen fixture per `SystemKind`**, loaded via `set_presets` and
captured by name (the pattern `beat.rs` already uses), keyed by an **exhaustive `match SystemKind`**
so the set is self-maintaining. Shipped presets drop pixel-pinning entirely; the three shipped
baselines are deleted. Fixtures live as TOML under `core/tests/fixtures/`, marked do-not-tune.

## Implementation phases

### Phase 1 — Per-system fixtures + repointed, self-maintaining golden test
- **Owner skill:** dev
- **Area:** core tests (`core/tests/golden.rs`, new `core/tests/fixtures/`, `core/tests/golden/`)
- **What:** Replace the shipped-preset `CASES` table with a fixture-per-system roster; author the
  fixtures; bless their baselines on WARP; delete the old shipped baselines.
- **Details:**
  - **Author one fixture TOML per `SystemKind` variant** under `core/tests/fixtures/`, named for the
    system (`fragment_field.toml`, `swarm.toml`, `parametric_curve.toml`, `lsystem.toml`,
    `star_pattern.toml`, and `reaction_diffusion.toml` **iff** that variant is present at
    implementation time — see Risks). Each is a **minimal, deterministic** preset that produces a
    non-trivial render (not blank, not a dot) under golden's existing `fixed_frame`, using constant
    or lightly-bound params so drift is catchable but the fixture never needs content tuning. Head
    each file with a comment: `# GOLDEN FIXTURE - do not tune; editing this invalidates the drift
    baseline (ADR-0023)`. Load them with `include_str!` + `Preset::from_toml_str`.
  - **Repoint `golden.rs`:** drop the `CASES` array (keyed by shipped preset name) and instead build
    the fixture roster from an **exhaustive `match SystemKind { ... }`** that maps each variant to
    its fixture TOML — no wildcard arm, so adding a `SystemKind` fails to compile until a fixture is
    added. Load the fixtures via `renderer.set_presets(...)`, then capture each by its fixture name
    (as `beat.rs` does). Baseline filename is the system name (`<system>.png`).
  - **Bless on WARP:** run `LMV_BLESS=1 cargo test -p lmv-core --test golden` on a Windows WARP box,
    eyeball each generated PNG (Plan 0013 Phase 8 habit — confirm each scene actually drew), and
    commit them under `core/tests/golden/`.
  - **Delete** the now-unused `core/tests/golden/aurora.png`, `warp.png`, `drift.png`.
  - Keep the existing tolerances (`MEAN_TOL = 0.02`, `MAX_OUTLIER = 48`) and the WARP-only skip
    (`headless()` from the ADR-0016 fix) unchanged.
- **Done when:** `cargo test -p lmv-core --test golden` is green on WARP against the per-system
  fixtures; every `SystemKind` variant present in the tree has exactly one fixture and one baseline;
  adding a throwaway `SystemKind` variant makes `golden.rs` fail to compile (verify once, then
  revert); no baseline or `CASES` entry references a shipped preset by name; the three old shipped
  baselines are gone. On an adapterless runner the test still skips cleanly (ADR-0016).

### Phase 2 — Confirm shipped-preset behavioral coverage + document the boundary
- **Owner skill:** dev
- **Area:** core tests (doc comments in `core/tests/golden.rs`, a short `core/tests/fixtures/README.md`)
- **What:** Verify the shipped roster is still guarded without a pixel-pin, and record the split so
  the coupling is not silently re-introduced.
- **Details:**
  - Confirm `sanity`, `reactivity`, and `animation` each iterate `default_presets()` (they do), so
    every shipped preset keeps a behavioral guard (coverage/spread, per-band reaction, motion). No
    code change expected — this is a verification step; if any of the three is found *not* to cover
    the full shipped roster, note it as a followup rather than expanding scope here.
  - Rewrite `golden.rs`'s module doc-comment to state the new contract: golden guards **engine
    rendering determinism** via **frozen per-system fixtures**, never shipped content; shipped
    presets are guarded behaviorally elsewhere; baselines are WARP-only and blessed on WARP.
  - Add `core/tests/fixtures/README.md` (a few lines): what these are, the do-not-tune rule, and
    that a new scene must add its fixture here (enforced by the exhaustive match).
- **Done when:** `golden.rs`'s doc-comment and `core/tests/fixtures/README.md` state the
  engine-vs-content split and the do-not-tune rule; no test pins a shipped preset by name for pixel
  comparison; the full test suite is green on WARP.

## Data shapes

No new Rust types, no C ABI change. Fixtures are ordinary preset TOML (same grammar as
`presets/*.toml`), e.g. illustrative:

```toml
# GOLDEN FIXTURE - do not tune; editing this invalidates the drift baseline (ADR-0023)
system = "star_pattern"
name   = "fixture_star_pattern"
# ... minimal deterministic params sufficient to draw a non-trivial frame ...
```

The golden roster becomes an exhaustive match (illustrative, ~10 lines):

```rust
// illustrative — dev owns the final shape
fn fixture_toml(system: SystemKind) -> &'static str {
    match system {
        SystemKind::FragmentField    => include_str!("fixtures/fragment_field.toml"),
        SystemKind::Swarm            => include_str!("fixtures/swarm.toml"),
        SystemKind::ParametricCurve  => include_str!("fixtures/parametric_curve.toml"),
        SystemKind::LSystem          => include_str!("fixtures/lsystem.toml"),
        SystemKind::StarPattern      => include_str!("fixtures/star_pattern.toml"),
        // a new variant fails to compile here until its fixture is added
    }
}
```

## Risks & open questions

- **`ReactionDiffusion` is in flight (Plan 0014).** The `SystemKind::ReactionDiffusion` variant may
  or may not be committed when this plan is implemented. Because the roster is an exhaustive match,
  golden will not compile without a fixture for every present variant. **Coordination:** whichever
  of Plan 0014 / this plan lands second owns adding the `reaction_diffusion` fixture + baseline;
  dev should check `SystemKind` at implementation time and cover exactly the variants that exist.
  This is the exhaustive-match property working as intended (no scene ships unguarded), not a defect.
- **Bless is WARP-tied and manual.** Baselines must be generated on Windows WARP or they drift
  (macOS skips per ADR-0016). Unchanged from today; the doc-comment in Phase 2 records it.
- **Interim red `main`.** Until this plan lands, `main` stays red from the `76a2fb4` shipped-baseline
  drift. This plan *resolves that durably* (the tuned shipped presets are no longer pinned). If
  `main` must be green sooner, a stopgap `LMV_BLESS=1` re-bless of `aurora`/`warp`/`drift` on WARP
  greens it in the interim — but those baselines are then deleted by Phase 1, so the stopgap is
  optional and only for sequencing.
- **Test-only, no hot-path risk.** Confined to `#[cfg(test)]` integration tests and fixture data; no
  per-frame, audio-callback, C ABI, or `ci.yml` surface is touched.

## What this plan does NOT do

- **Does not change production render code, the `Scene` trait, or the C ABI.**
- **Does not touch `ci.yml`** — golden stays in the default `nextest` set, WARP-only per ADR-0016.
- **Does not add pixel-pinning back onto shipped presets** at any tolerance (ADR-0023 Alternative B,
  rejected) — shipped presets are guarded only behaviorally.
- **Does not re-bless or redesign the shipped presets themselves** — the `76a2fb4` field/flock look
  is the `preset-author`/`dev` owner's call; this plan only stops golden from pinning it.
- **Does not implement `ReactionDiffusion`** (Plan 0014); it only notes the fixture-coordination
  point.

## Followups (after this lands)

- If Phase 2 finds any shipped preset **not** covered by `sanity`/`reactivity`/`animation`, open a
  small followup to extend the behavioral floors so no shipped preset is wholly unguarded once the
  golden pixel-pin is removed.
