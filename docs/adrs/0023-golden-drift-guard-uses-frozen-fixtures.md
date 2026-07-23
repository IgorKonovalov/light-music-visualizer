# ADR-0023 — The golden drift guard renders frozen per-system fixtures, not shipped presets

> **Status:** proposed
> **Date:** 2026-07-23
> **Related plan(s):** 0022-golden-fixtures-decouple-content (proposed); the golden capture
> tests it repoints originate in Plan 0013
> **Related ADRs:** [0016](0016-gpu-tests-opt-in-ci-scope.md) (headless GPU tests skip when no
> adapter — golden's WARP-only execution); [0017](0017-preset-author-skill-lane.md) (the content
> lane that tunes shipped presets)

## Context

`core/tests/golden.rs` guards against **unintended rendering drift**: it renders a small matrix
headless on the Windows WARP software adapter and compares each frame to a committed baseline PNG
within a mean + max-outlier tolerance, blessing with `LMV_BLESS=1`. The intent is an *engine*
regression alarm — a shader edit or scene-math change that silently perturbs output should trip it.

But the baselines are pinned to three **shipped, curated presets** (`Aurora`, `Warp Drive`,
`Drift`), looked up by name in the default roster. Those presets are exactly what the
`preset-author` lane ([ADR-0017](0017-preset-author-skill-lane.md)) exists to tune — "make it more
organic", "make field and flock distinct" is its *normal work*. So every intentional content tune
trips an engine-drift alarm and reds CI until someone re-blesses. This is a coupling of two things
with **opposite change cadences**: engine rendering (stable, should be pinned) and shipped content
(churny, should not be). It surfaced concretely when commit `76a2fb4` ("make field and flock
presets distinct") moved all three baselines far past tolerance (mean 0.15–0.25 vs 0.02).

Worse, the coupling crosses lanes: a `preset-author` doing their job reds a `dev`-owned test they
may not know exists. And it is a brittle seam in a second way — golden only runs on Windows WARP
(macOS skips per ADR-0016), so a re-bless must happen on WARP or it drifts again; the baseline is
implicitly tied to the machine that blessed it.

The shipped presets are not the right *drift* fixtures, but they are not unguarded either: the
`sanity`, `reactivity`, and `animation` integration tests already defend them **behaviorally**
(coverage, quadrant spread, per-band reaction, animation floors) — assertions that survive content
tuning by design. Only the pixel-exact golden test was pointed at content it shouldn't pin.

This is a real decision because there is a defensible opposite (keep golden on shipped presets and
accept the re-bless tax as a "confirm you meant it" gate), so the choice and its price are worth
recording.

## Decision

The golden drift guard will render **test-only frozen fixtures — one per `SystemKind` variant —
never shipped presets.** Each fixture is a minimal, deterministic preset that exercises its scene,
authored as a TOML file under `core/tests/fixtures/`, marked *do-not-tune*, and loaded into the
renderer via `set_presets` before capture (the pattern `beat.rs` already uses). The fixture roster
is keyed by an **exhaustive `match SystemKind`**, so adding a new system fails to compile until its
fixture exists — the per-system coverage is self-maintaining and the coupling cannot silently
regress.

The shipped/curated presets **drop all golden pixel-pinning** and rely solely on the existing
behavioral floors (`sanity` / `reactivity` / `animation`). Content tuning therefore never trips the
engine-drift guard again. The three shipped-preset baselines (`aurora.png`, `warp.png`,
`drift.png`) are deleted.

Golden's WARP-only execution is unchanged (ADR-0016 owns that); fixtures are blessed on WARP.

## Consequences

### Positive
- **The coupling is cut.** `preset-author` can tune the shipped roster freely; golden never reds
  on an intended content change. The cross-lane brittle seam is gone.
- **The engine guard still fires**, now per *system* — closing today's gap (the three line-family
  systems `parametric_curve` / `lsystem` / `star_pattern` had **zero** golden coverage) rather than
  only two fragment_field presets and one swarm.
- **Self-maintaining coverage.** The exhaustive `match SystemKind` forces every new scene to add a
  drift fixture in the same change that adds the variant — no scene ships unguarded.
- **Landing this also greens `main`** from the `76a2fb4` drift, because the tuned shipped presets
  are no longer pinned; no separate re-bless of `aurora`/`warp`/`drift` is needed.

### Negative
- **A shipped preset going visually wrong** (e.g. a tune that accidentally darkens Aurora to
  near-black) is no longer caught by *pixel* comparison — only by the behavioral floors. Accepted:
  the floors already assert "not blank, not a dot, reacts to a band, animates", which is the class
  of regression that matters for content; exact-pixel pinning of content was never its job.
- **Fixtures are a small maintenance surface** — 5–6 tiny TOMLs plus their PNGs. Mitigated by their
  being minimal and explicitly frozen; they change only when the *engine* legitimately changes
  output, which is exactly when a human eyeball (the bless step) is wanted.
- **WARP-only bless remains** a manual, machine-tied step (unchanged from ADR-0016).

### Neutral
- No production code, no C ABI, no `ci.yml` change — the decision lives entirely in
  `core/tests/golden.rs`, the new `core/tests/fixtures/`, and the deleted baselines.
- New systems in flight inherit the obligation: the `ReactionDiffusion` variant (Plan 0014) adds
  its fixture when it lands, forced by the exhaustive match — a deliberate cross-plan coordination
  point, not a silent gap.

## Alternatives considered

### Alternative A — Keep golden on shipped presets, accept the re-bless tax
Leave the baselines pinned to curated presets and treat the CI red as a "confirm you meant it"
gate: on any intentional tune, re-bless and eyeball. Rejected because it welds a stable guard to
churny, cross-lane content — the tax lands on the `preset-author` lane for doing its normal work,
and the alarm cries wolf on every intended change, which trains people to bless without looking.

### Alternative B — Also keep a loose pixel-pin on shipped presets
Point golden at fixtures *and* keep a wide-tolerance pin on the shipped presets to catch gross
content regressions. Rejected because it re-introduces exactly the coupling being removed (any tune
past the loose tolerance still reds CI and demands a re-bless), for a regression class the
behavioral floors already cover.

### Alternative C — Per-idiom fixtures (~3) instead of per-system
Pin one fixture per render idiom (fragment / lines / points) since the three line-family systems
share a renderer. Rejected as under-coverage: those systems feed the shared line renderer through
*different* geometry generators (parametric curve vs L-system vs Hankin star), so a regression in
one generator would not show if only another line system were pinned. Per-system fixtures are cheap
(tiny PNGs) and catch generator drift.

### Alternative D — Embedded fixture consts in the test, or reuse a `default_presets()` subset
Keep fixtures as `from_toml_str` constants inside `golden.rs`, or just reference shipped presets by
name. Rejected: embedded consts bury the fixtures in test code and make them hard to review/edit as
presets; reusing `default_presets()` *is* the coupling this ADR removes. TOML files under
`core/tests/fixtures/` are authored in the same grammar, diff readably, and are visibly out of the
shipped `presets/` set.
