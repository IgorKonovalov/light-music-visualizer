# ADR-NNNN — <decision title>

> **Status:** proposed | accepted | superseded by ADR-NNNN
> **Date:** YYYY-MM-DD
> **Related plan(s):** NNNN-foo (if applicable)

## Context

What forces are at play? What constraint or tradeoff made this a *decision* — a thing that could
reasonably go either way — rather than a no-brainer? Two to four short paragraphs. Cite concrete
facts: a real-time budget, a platform limitation (macOS loopback), an SDK constraint (foobar is
C++), a benchmark. ADRs are credible because of the context, not the prose.

## Decision

One paragraph, active voice, present tense.

> We will render through wgpu, targeting Metal on macOS and DX12/Vulkan on Windows, and write
> all scene code against the wgpu abstraction rather than a per-backend path.

If the decision has nuance ("use X unless Y"), capture it here, not in a footnote.

## Consequences

### Positive
- What this unlocks. New capabilities, simplified paths.

### Negative
- What this costs. **The most important section — be honest.** (An FFI seam to maintain, a
  younger API, an unsolved platform gap.)

### Neutral
- Things that change but aren't clearly better or worse. Optional.

## Alternatives considered

For each rejected alternative, one paragraph: what it was, the one decisive reason it lost. More
than three usually means padding.

### Alternative A — <name>
Why rejected.

### Alternative B — <name>
Why rejected.

## Notes

Free-form. Links to benchmarks, prototypes, prior discussion. Skip if empty.
