# ADR-0004 — Living behavioral-spec layer: seed two contracts, no gate, no ritual yet

> **Status:** accepted
> **Date:** 2026-07-21
> **Related plan(s):** none (architect-owned docs; landed with this ADR)

## Context

This is a contract-heavy, determinism-heavy project. The CLAUDE.md non-negotiables —
the sacred audio callback, pure-function DSP, the source-agnostic core, the versioned C
ABI, the ring-buffer decoupling between audio and render — are *behavioral invariants*,
not style preferences. Today they live in three places: prose bullets in CLAUDE.md,
per-module doc headers (`ffi.rs`, `audio.rs`, `dsp/mod.rs`), and the reasoning inside
ADRs. None of those is a per-subsystem behavioral contract that says, in checkable form,
"the SPSC ring MUST be data-race-free" or "`lmv_push_samples` MUST NOT allocate."

The sibling repo `market-analyzer` solved the same gap with a *living-spec layer*
(its ADR-0106): one Markdown behavioral contract per core subsystem — Invariants
(MUST-statements) plus `WHEN/THEN` scenarios — cross-linked to a governing ADR and the
source that enforces it, kept honest by two mechanisms: a `Reconciled-through: Plan NNNN`
line bumped at each plan's close, and a structural freshness gate (`specs --check`) that
fails CI on missing sections or dangling plan references.

The question is dosage, not direction. `market-analyzer` adopted the full apparatus at
~110 shipped plans; this repo is at Plan 0005 / ADR-0003. The full gate-plus-ritual
machinery is exactly the ceremony this repo deliberately trimmed when it adapted the
harness down to two skills. But the *idea* — a durable, per-subsystem behavioral truth
that outlives the plan that created it — earns its keep here precisely because the
invariants are load-bearing (a leak in the source-agnostic rule or a race in the ring is
a real defect, not a lint nit).

## Decision

We adopt the living-spec layer in its **minimal, lazy form**: a `docs/specs/` directory
holding exactly two seed contracts now — the **C ABI** (`0001-c-abi.md`) and the
**ring + DSP determinism** contract (`0002-ring-determinism.md`) — each stating its
invariants and `WHEN/THEN` scenarios, cross-linked to its governing ADR(s) and the source
that enforces it. These are the two highest-value invariants in the codebase (the C ABI is
a compiled cross-language contract; the ring's data-race-freedom is literally what deferred
Plan 0002 Phase 5 / Plan 0005 exists to prove).

We deliberately **do not** adopt the enforcement scaffolding yet: **no `specs --check` CI
gate**, and **no mandatory `Reconciled-through` bump ritual** in the close ceremony. Each
spec header carries an informational `Reconciled-through: Plan NNNN` line so the layer is
forward-compatible with a gate if we ever add one, but reconciliation is **best-effort
architect judgment at plan close**, not a machine-checked step. Specs are added **lazily**
as future plans touch a subsystem worth contracting; the two seeds are the intended
starting ceiling, not a target to pad toward. These are architect-authored documents (like
ADRs and diagrams), outside the `dev`/`human` owner-tag vocabulary.

The escalation trigger is explicit: **when the spec count outgrows what a human reader can
eyeball for freshness (roughly five or more specs), or when a stale spec actually misleads
someone**, we revisit adding the `dev`-owned freshness gate and the formal reconcile step
as their own ADR + plan. Until then, lightweight wins.

## Consequences

### Positive
- Captures the two load-bearing invariants (C ABI contract, ring/DSP determinism) as
  durable per-subsystem contracts that outlive the plans that created them — something no
  current artifact does (CLAUDE.md is orientation, module headers are local, ADRs are
  rationale).
- Near-zero added ceremony: two Markdown files and a README, no new dependency, no CI job,
  no mandatory close-ceremony step. Diffs cleanly, renders in GitHub/VS Code.
- Forward-compatible: the `Reconciled-through:` line and the exact `## Invariants` /
  `## Scenarios` headings mean that if we later add the gate, the seed specs already satisfy
  it without a reformat.
- Gives the deferred Miri work (Plan 0005) a home for its result — "the SPSC ring is
  UB-free" becomes an invariant in `0002-ring-determinism.md`, not a fact buried in a
  closed plan.

### Negative
- **A spec that silently goes stale is worse than no spec — it lies with authority.**
  Without the freshness gate, only architect discipline at close keeps the two specs honest.
  We accept this because two specs are eyeball-checkable, and we name the escalation trigger
  (add the gate before the count makes manual review unreliable).
- **New maintenance surface, however small.** The close ceremony *should* glance at whether
  a just-closed plan changed C-ABI or ring/DSP behavior and, if so, reconcile the spec —
  a soft step that can be forgotten precisely because it isn't gated.
- **Partial overlap with CLAUDE.md and module headers.** The specs are more granular
  (checkable MUST-statements and scenarios vs. orientation prose), but a reader now has two
  places that describe the same invariant; they must not drift apart.

### Neutral
- The layer sits outside the `dev`/`human` owner vocabulary — it is architect-authored,
  the same shape as ADRs and diagrams. No plan owns it; this ADR lands the seeds directly.
- `docs/specs/` (behavioral intent) and future generated API reference or the C header
  `core/include/lmv_core.h` (mechanical surface) would be complementary, not substitutes.

## Alternatives considered

### Alternative A — Adopt the full apparatus now (gate + reconcile ritual)
Mirror `market-analyzer` exactly: per-subsystem specs, a `dev`-owned `specs --check` CI
gate, and a mandatory `Reconciled-through:` bump in every close ceremony. Rejected as
premature at Plan 0005 / ADR-0003: `market-analyzer` earned that machinery at ~110 plans
where manual freshness review had become unreliable. Here it re-adds precisely the ritual
this repo trimmed, to guard two files a human can check by eye. The gate is the right *next*
step, recorded above as an explicit escalation trigger — not the right *first* step.

### Alternative B — Defer entirely (no specs)
Let CLAUDE.md's non-negotiables and the module doc headers carry the invariants, and revisit
if a subsystem's contract ever gets too complex for prose. Rejected because it leaves the two
highest-value invariants — the cross-language C ABI contract and the ring's data-race-freedom
— without a single checkable home, exactly as the Miri deferral (Plan 0002 Phase 5 → Plan
0005) is about to produce a durable invariant with nowhere to record it. Taking the idea in
its cheapest form costs two files and captures that value now.

## Notes
- Concept and precedent: `market-analyzer` ADR-0106 (living-spec layer) and its
  `docs/architecture/specs/`. This ADR borrows the idea in its minimal form and declines the
  tool/gate/ritual, mirroring how this repo's two-skill harness was adapted down from the
  larger one.
- The two seed specs land with this ADR: `docs/specs/0001-c-abi.md` (governed by ADR-0003,
  ADR-0001) and `docs/specs/0002-ring-determinism.md` (governed by CLAUDE.md non-negotiables,
  Plan 0005). See `docs/specs/README.md` for the layer's posture and the add-a-spec steps.
