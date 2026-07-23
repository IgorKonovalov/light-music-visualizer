# ADR-0016 — Headless GPU-capture tests skip when no adapter is present (keep GPU out of the CI contract)

> **Status:** proposed
> **Date:** 2026-07-23
> **Related plan(s):** 0017-ci-green-advisory-and-gpu-tests

## Context

`ci.yml` opens with an explicit scope statement: *"GPU rendering and live audio are out of
CI scope — those checks stay manual."* The intent is that CI proves the code **builds, lints,
and its GPU-free logic is correct** on Windows and macOS, not that a frame actually renders on
a runner.

Plan 0013 later added headless-capture tests (`headless_captures_a_non_black_frame`,
`capture_preset_is_deterministic_and_animates`, and a sibling) that build a real surfaceless
`Renderer` via `new_headless(prefer_software: true)` and assert on captured pixels. On Windows
these pass because DX12 exposes **WARP**, a guaranteed software adapter — the test comment
explicitly banks on this ("`prefer_software` (WARP on DX12) keeps it reproducible on any
adapter"). They therefore ran green in the default `cargo nextest run` set on both runners and
nobody noticed they were, in fact, GPU tests running in CI.

The macOS runner image then stopped exposing a usable Metal adapter to the headless test
process: `request_adapter` returns `RequestAdapterError` → `RenderError::RequestAdapter`, the
`.expect()` panics, and CI goes red (run 29985131075). Metal has **no software-fallback
adapter** (unlike DX12's WARP or Vulkan's lavapipe), so `force_fallback_adapter: true` cannot
save it. The failure is environmental — a runner-image change — not a code regression, and it
exposed a latent contradiction: these tests depend on a GPU adapter, which CI's own contract
says it does not guarantee.

This is a real decision because the tests have genuine value where an adapter exists (Windows
WARP is deterministic and free), so "delete them" is wrong — but letting adapter presence, an
uncontrolled property of the runner image, decide whether `main` is green is also wrong.

## Decision

Headless GPU-capture tests will **treat a missing adapter as "skip", not "fail"**: they match
the constructor result and, on `Err(RenderError::RequestAdapter(_))` specifically, print a
one-line skip notice to stderr and return without asserting; any other error still panics, and
when an adapter is present the assertions run in full. The tests stay in the default
`cargo nextest run` set, so they keep running **for real on Windows WARP** every push and merely
go quiet on an adapterless runner. This holds the line on the CI contract — a green CI never
depends on GPU-adapter availability — while preserving the coverage on the one runner that
guarantees a deterministic software adapter.

The skip keys off the specific `RequestAdapter` variant, not any error, so a genuine
device/pipeline/capture failure on an adapter-equipped runner still fails loudly.

## Consequences

### Positive
- `main` no longer goes red from a runner-image change that removes the Metal adapter.
- CI's stated contract ("GPU out of scope") becomes true again: no check depends on a GPU
  actually being present.
- Real coverage is retained where it is deterministic and free — Windows WARP runs the full
  assertions on every push.
- The pattern is reusable: any future headless-GPU test adopts the same match-and-skip shape.

### Negative
- A skipped test is a **silent no-op** on that runner. If Windows WARP ever also disappears,
  both runners would skip and the capture path would be unverified in CI with only a stderr
  line to show it. Mitigation: the skip prints a visible `skipped: ...` notice, and Windows WARP
  is a far more stable guarantee than a runner's real GPU.
- Slight honesty tax: a "passing" run may have asserted nothing on macOS. This is the accepted
  price of keeping GPU out of the hard CI contract; on-device visual QA remains manual (the
  Plan 0013 `shot` CLI and the standing on-device checklist own that).

### Neutral
- No new `RenderError` variant and no CI-workflow change — the existing `RequestAdapter` variant
  and the existing single `nextest` step carry the decision entirely in test code.

## Alternatives considered

### Alternative A — `#[ignore]` the GPU tests, run them only on Windows via `--run-ignored`
Mark the headless tests `#[ignore]` and add a Windows-only CI step that runs the ignored set
against WARP; macOS never touches them. This is arguably the most literal reading of the CI
contract, but it adds per-OS workflow plumbing, splits the test invocation into two commands,
and makes the tests invisible in the normal `nextest` run on the machine where a developer is
most likely to notice a regression (their own box, which usually *has* an adapter). Rejected for
the extra CI surface and the loss of run-by-default coverage where an adapter exists.

### Alternative B — Delete the headless-capture tests from CI entirely (manual only)
Fully honor "GPU out of CI scope" by removing these from any automated run. Rejected because it
throws away deterministic, free Windows-WARP coverage that catches real capture/determinism
regressions — the tests earn their place wherever an adapter is present; only their *hard
failure on absence* is the problem.

### Alternative C — Force a software adapter on macOS too
There is no software Metal adapter to force; `force_fallback_adapter: true` already fails on the
runner. Not a viable option, listed only to record that it was checked.
