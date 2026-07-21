# NNNN — <short title>

> **Status:** draft | in-progress | done | abandoned
> **Created:** YYYY-MM-DD
> **Owner skill(s):** <dev | human> (list all that appear in phase owner tags below)
> **Related ADRs:** NNNN-foo (link if any)

## TL;DR

One paragraph. What we're building, why, and the first user-visible behavior. A reader who
reads only this should be able to restate the decision in a sentence.

## Context & problem

What forces drove this? Reference the user's request. Be specific about the *problem*, not the
chosen *solution* — the rest of the doc handles that.

## Decision

The chosen approach in one paragraph, active voice. If picked from options during the interview,
name the rejected ones in one sentence: "We rejected B (X) because … and C (Y) because …".

## Architecture diagram

```mermaid
flowchart LR
    %% Replace with a real diagram. Use subgraph blocks for boundaries
    %% (what's inside core/ vs the shells vs external: foobar, OS audio).
    A[Component A] --> B[Component B]
```

## Implementation phases

Each phase ships as its own commit. `dev` runs all phases in one session — no architect review
between phases; the architect reviews the whole plan once at the end. Order phases so the first
is valuable on its own (a walking skeleton), not just plumbing.

**Every phase MUST carry a single `**Owner skill:**` line** with exactly one value: `dev` (all
code) or `human` (a task only the user can do). The tag is machine-readable — `dev` reads it at
the start of each phase and stops/surfaces on a `human` phase. Missing tags fail Mode 4 review.

### Phase 1 — <name>
- **Owner skill:** <dev | human>
- **What:** One sentence on what this phase produces.
- **Files touched:** Rough list — `core/src/dsp/fft.rs`, `standalone/src/main.rs`, etc.
- **Done when:** Concrete acceptance — "`cargo run -p standalone` shows spectrum bars reacting
  to system audio at a stable frame rate". For test files, phrase the behavioral claim the spec
  defends ("sine wave produces energy in exactly one FFT bin"), not "the test passes".

### Phase 2 — <name>
…

## Data shapes

If this plan introduces new structs (an `AnalysisFrame`, a config record, the C ABI signature),
pin them down here. A short illustrative Rust/C sketch is fine — label it illustrative.

```rust
// illustrative — not the final interface
pub struct AnalysisFrame {
    pub spectrum: Vec<f32>,  // normalized, log-frequency bins
    pub onset: f32,          // 0..1 onset strength this frame
    pub beat: bool,          // beat detected this frame
}
```

## Risks & open questions

Bullet list. Each item: what could go wrong, what we'd do about it. Don't pretend everything is
solved. Call out real-time hazards (allocation in a hot path, ABI lifetime questions) explicitly.

## What this plan does NOT do

Cut the scope explicitly. Tempting-to-bundle things that are out of scope; reference future plans
by name if you can.

## Followups (after this lands)

Track followups as a list so they don't get lost. Empty at draft time is fine.
