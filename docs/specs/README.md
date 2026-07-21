# Living behavioral specs

One hand-authored **behavioral contract per core subsystem** — the invariants and
`WHEN/THEN` scenarios that are true of the *running system today*. This layer is the one
idea borrowed from the sibling repo's living-spec system
([ADR-0004](../adrs/0004-living-behavioral-spec-layer.md)) in its **minimal, lazy form**:
the two highest-value contracts, and none of the enforcement machinery yet.

## What lives here, and what doesn't

Three artifacts answer three different questions; keep them distinct:

| Artifact | Question | Lifecycle |
|----------|----------|-----------|
| **Plans** (`plans/`) | *What are we building next?* | Expire → `git mv` to `plans/done/` at close |
| **ADRs** (`adrs/`) | *Why did we choose this over the alternatives?* | Append-only; superseded, never edited |
| **Specs** (here) | *What does the system do now, by behavior?* | Living — reconciled best-effort at close |
| `core/include/lmv_core.h` | *What is the mechanical C surface?* (signatures) | Mirrors `core/src/ffi.rs` |

A spec is **behavioral intent**, not mechanical surface: "`lmv_push_samples` MUST NOT
allocate or block" belongs here; the exact C signature of `lmv_push_samples` belongs in the
header. Specs are per *subsystem behavioral contract*, not per file — a subsystem with no
non-obvious invariant gets no spec. This layer also overlaps CLAUDE.md's non-negotiables and
the module doc headers on purpose: the specs are the *granular, checkable* form (MUST-
statements + scenarios); CLAUDE.md is orientation. They must not drift apart.

## Index

| Spec | Subsystem | Governing ADRs |
|------|-----------|----------------|
| [0001-c-abi.md](0001-c-abi.md) | The versioned `extern "C"` surface the foobar plugin links | 0003, 0001 |
| [0002-ring-determinism.md](0002-ring-determinism.md) | SPSC ring seam + pure-function DSP determinism | 0001 (+ CLAUDE.md non-negotiables); Plan 0005 |

## Posture: minimal and lazy (no gate, no ritual yet)

Per ADR-0004, this layer is deliberately lightweight:

- **No CI freshness gate.** There is no `specs --check`. Two specs are eyeball-checkable.
- **No mandatory reconcile ritual.** Each spec header carries an informational
  `Reconciled-through: Plan NNNN` line (so the layer is forward-compatible with a gate if we
  ever add one), but bumping it is **best-effort architect judgment at a plan's close**, not
  a machine-checked close-ceremony step. When a plan's close changed C-ABI or ring/DSP
  behavior, the closer *should* glance at the relevant spec and reconcile it — a soft step.
- **Specs are added lazily** as future plans touch a subsystem worth contracting. The two
  seeds are the intended starting ceiling, not a target to pad toward.

**Escalation trigger (ADR-0004):** when the spec count outgrows eyeball freshness review
(roughly five or more), or a stale spec actually misleads someone, revisit adding the
`dev`-owned freshness gate and a formal reconcile step — as its own ADR + plan. Until then,
lightweight wins.

## Adding a spec

1. Create `docs/specs/NNNN-<subsystem>.md` (next number above), with a header carrying
   **Subsystem / Source / Reconciled-through / Governing ADRs**, then `## Invariants`,
   `## Scenarios`, and `## Known gaps`.
2. Write invariants as MUST-statements a maintainer can hold the code to; write scenarios as
   observable `WHEN/THEN` behavior. Ground each claim in an ADR and, where useful, the source
   file that enforces it.
3. Add a row to the index above.
4. Only add a spec when the subsystem has a non-obvious behavioral contract worth pinning —
   not for every module.
