# ADR-0017 — A third skill lane: `preset-author` (preset content, not engine code)

> **Status:** proposed
> **Date:** 2026-07-23
> **Related plan(s):** none yet (the skill file is authored as a user-gated step, not a `dev` plan — see Notes)
> **Related:** [ADR-0002](0002-layered-preset-architecture.md) (the preset layers this skill authors),
> [ADR-0007](0007-line-geometry-generators.md) (the `[curve]`/`[generator]` config it also writes),
> [Plan 0010](../plans/done/0010-line-geometry-scenes.md) (the schema whose settling was the deferral gate — now closed),
> [Plan 0013](../plans/done/0013-headless-scene-capture.md) (the `shot` CLI this skill renders through),
> [Plan 0015](../plans/0015-preset-dir-override-and-live-iteration.md) (tightens the edit-see-live loop; a soft, not hard, dependency)

## Context

The project runs a deliberate **two-skill harness** (`.claude/skills/`): `architect` owns `docs/`
(plans, ADRs, diagrams, reviews) and `dev` owns all code (`core/`, `standalone/`,
`plugin-foobar/`). The hard rule is that `architect` designs and `dev` builds, never inverted; the
value of the split is the clean-context boundary between deciding and doing.

A **preset is neither of those artifacts.** After [ADR-0002](0002-layered-preset-architecture.md)
(layers 1-2, delivered by Plan 0003) and [ADR-0007](0007-line-geometry-generators.md) (delivered by
Plan 0010), a preset is *content*: a `.toml` file of named params, pure expression bindings over the
audio vocabulary (bass/mid/treb, beat, tempo), and structural `[curve]`/`[generator]` config tables.
Authoring one writes no Rust — it composes existing engine capability into a look. That makes it a
genuinely **third artifact type**, orthogonal to `architect`'s design docs and `dev`'s engine code.
Folding it into `dev` blurs a real distinction (composing content vs. changing the engine) and gives
up the fresh-context boundary; leaving it unowned means preset work has no home lane.

Three forces make this the right time to add the lane, and shape its edges:

- **The vocabulary is now stable.** This was the explicit reason to defer (agreed 2026-07-22): Plan
  0010 was still moving the preset schema across its phases (new `[curve]`/`[generator]` tables,
  named params), and building a skill against a shifting grammar risked rework. **Plan 0010 has now
  closed**, so the authoring surface the skill targets is settled.
- **Render-and-verify tooling exists.** Taste in presets is not reusable as prose — it needs
  side-by-side artifacts to pick from (the user's standing "design by concrete examples" preference).
  [Plan 0013](../plans/done/0013-headless-scene-capture.md) landed the headless `shot` CLI
  (`--preset`/`--presets`/`--preset-file`/`--all` contact sheet), so the skill can render its own
  drafts into stills without a running window.
- **The hard boundary must hold.** Anything a preset *can't* express today — a new render system, a
  new named param, a new grammar capability — is engine work. The skill must route that back to
  `architect` + `dev` rather than reaching into Rust, exactly as `dev` never writes plans.

## Decision

We will add a **third standalone skill, `preset-author`**, as a peer lane alongside `architect` and
`dev`. It owns **preset content only** — `.toml` files, expression bindings, and `[curve]`/`[generator]`
config — and **never engine Rust**. Its boundary mirrors the existing hard split: any need for a new
system, named param, or grammar capability is handed back to `architect` + `dev`, not worked around
in a preset.

Its default output is **user-directory presets** (the seeded per-user preset dir, or a
`presets/`-style folder), and it **flags the strongest drafts as curation candidates** — but
*curation* (embedding a preset into the shipped set) stays a **`dev`/`human` close-out**, because
embedding touches Rust in two coupled spots (the `EMBEDDED` array and its `[(&str,&str); N]` length
type in `core/src/preset/mod.rs`, and the count assert in `core/tests/preset.rs`). The skill proposes;
`dev` embeds.

The skill **validates by rendering its own drafts through the `shot` CLI** (`cargo run --example shot`
— using `dev`-built tooling, not writing engine code, the same way running tests is not writing the
test framework) into side-by-side stills for the user to pick from. This is a soft dependency on
Plan 0015: the loop works today via `shot`'s `--presets`/`--preset-file` flags against the landed
Plan 0013 harness; Plan 0015's `LMV_PRESET_DIR` override + tightened hot-reload will make the
edit-see-live loop tighter but is not a prerequisite.

CLAUDE.md's workflow table gains a third row; the `docs/` two-skill diagrams and the `architect`/`dev`
skill files that describe "the whole ecosystem" are updated to name the third lane.

## Consequences

### Positive

- Preset work gets a home lane with the right focus: composing existing capability into looks,
  fluent in the expression grammar and the `[curve]`/`[generator]` config, without the cognitive load
  of engine internals.
- The clean-context boundary is preserved and extended: `preset-author` drafts and self-verifies;
  `dev` embeds the winners; `architect` decides when a preset need is really an engine need. Each
  handoff stays a fresh-context seam, the same property that makes the existing split valuable.
- Render-and-verify is built into the lane — the skill produces the side-by-side stills the user
  picks from, matching how design decisions are already made here.
- Zero engine risk: the skill cannot touch `core/`/`standalone/`/`plugin-foobar/` source, so a bad
  preset is a bad `.toml`, never a regression in the engine or the C ABI.

### Negative

- **A third lane is more harness surface to keep coherent.** Three boundary rules instead of two;
  CLAUDE.md, the diagrams, and the sibling skill files all now have to describe a three-lane
  ecosystem and stay in sync. This is the price of the extra artifact type.
- **The content/code boundary has a genuine grey edge: curation.** Promoting a user-dir preset into
  the shipped set is a `dev`/`human` step by design, so the strongest presets need a deliberate
  handoff — the skill flags, it does not embed. If that handoff is skipped, good presets sit
  un-shipped in a user dir.
- **The skill runs `cargo` to self-verify.** Invoking `shot` is using tooling, not writing code, but
  it does mean the lane isn't purely textual — it builds and runs the standalone example. We accept
  this as the cost of render-and-verify; the alternative (author blind, hand off every render) is
  worse.

### Neutral

- **The skill file itself is user-gated to create.** The auto-mode classifier denies edits under
  `.claude/skills/**`, so standing up `preset-author/SKILL.md` is a step only the user can commit —
  the same posture as any skills-dir change today. This ADR records the decision; authoring the file
  is a separate, manual act (see Notes).
- The foobar plugin path is unaffected: presets are shared on-disk content both frontends already
  read; nothing about this lane touches the C ABI (still v3) or the plugin shim.

## Alternatives considered

### A — A **mode of `dev`** (a preset-authoring mode inside the existing implementer skill)

Rejected. `dev` is the *code* lane; giving it a preset mode blurs the composing-content vs.
changing-the-engine distinction that motivates a separate lane in the first place, and — because it's
the same skill in the same session — forfeits the fresh-context boundary between authoring content
and modifying the engine that could render it. It also makes the boundary rule ("a new param means
engine work") a soft internal note rather than a hard cross-skill handoff. A preset is a different
artifact type, not a `dev` sub-task.

### B — A **reference document only** (a preset-authoring guide in `docs/`, no skill)

Rejected. Preset taste is variable and iterative — it lives in rendering drafts, comparing stills,
and tuning bindings, not in a static how-to. A doc captures the grammar (and one exists, the Plan
0010 `presets/README.md` authoring note), but it is not a *capability*: it can't render-and-verify, it
can't iterate toward a look, and it can't hold the boundary rule as an enforced lane. The reusable
thing here is an agent that authors and self-checks, not a page that explains how.

## Notes

- **Acceptance trigger.** This ADR is `proposed` until the `preset-author` skill file is authored and
  placed under `.claude/skills/preset-author/` (the user-gated step above) and CLAUDE.md's workflow
  table + the ecosystem diagrams/skill files are updated to name the third lane. Because the
  implementation is a single user-gated authoring act rather than a phased `dev` plan, there is no
  paired plan to close against — flip this to `accepted` (and update `docs/adrs/README.md`) once the
  skill exists and the docs name it.
- **Soft dependency on Plan 0015.** The render-and-verify loop runs today through the landed Plan
  0013 `shot` CLI (`--presets`/`--preset-file`). Plan 0015's `LMV_PRESET_DIR` + tightened hot-reload
  tightens the edit-see-live loop but does not gate the skill.
- **Curation touch points (why embedding stays `dev`/`human`).** Promoting a preset into the shipped
  set edits two Rust spots — the `EMBEDDED` array + its `[(&str,&str); N]` length type in
  `core/src/preset/mod.rs`, and the count assert in `core/tests/preset.rs` — plus the on-disk
  `presets/` file. The skill flags candidates; the embed is a `dev` change with the usual guard-test
  update.
- Prior discussion: deferral recorded 2026-07-22 (agreed shape locked, revisit gated on Plan 0010's
  close); re-opened 2026-07-23 now that Plan 0010 has closed.
