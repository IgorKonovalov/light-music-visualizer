# ADR-0020 — Preset expression grammar v2: branching, math functions, a tempo variable, and soft typo warnings

> **Status:** proposed
> **Date:** 2026-07-23
> **Related plan(s):** 0019-preset-grammar-v2
> **Supplements:** [ADR-0002](0002-layered-preset-architecture.md) (does not supersede — the layered architecture stands; this refines layer-1's expression vocabulary)

## Context

[ADR-0002](0002-layered-preset-architecture.md) fixed the preset expression language as a small, pure, allocation-free, total (never-panics) arithmetic over a handful of analysis variables — and its own "Negative" section named the price: *"We own two languages … presets written today must keep working; the host API needs versioning discipline like the C ABI."* We now pay some of that price deliberately.

A grammar exploration (driven by the [`preset-author` skill lane](0017-preset-author-skill-lane.md), whose charter is to hand every grammar wall back to the architect) mapped the actual shipped surface against the code, and found it narrower than the docs and even ADR-0002's prose implied:

- **7 variables** (`bass mid treb onset beat bar time`) — notably **no `tempo`**, even though the DSP already computes BPM (`AnalysisFrame.bpm`, `dsp/tempo.rs`) and merely fails to plumb it into `Variables`.
- **7 functions** (`sin abs floor min max clamp lerp`) — no `cos` (authors write `sin(x + 1.5708)`), no `sqrt`/`pow`/`smoothstep`/`mod`.
- **4 operators** (`+ - * /`), no comparison/logical/conditional, no constants (no `pi`/`tau`).
- A **silent footgun**: an unknown *parameter* name is ignored (each scene's `set_param` ends in `_ => {}`, and `schema.rs` compiles every bound name blindly), so a typo compiles fine and does nothing — even though an unknown *system*, *function*, or *variable* is already a hard load error. That inconsistency is the bug.

One force constrains the fix hard; a second does not yet apply. **Real-time safety** (NFR §5, the hot-path panic-denial pragma on `preset/expr.rs`) binds absolutely: `eval` runs per parameter per frame, so every addition must stay total and allocation-free — no operation may panic on any input. **Backward compatibility does not yet apply**: the app is pre-1.0 (0.3.0) and in active development, and preset-format stability starts at 1.0.0. ADR-0002's negative — *"presets written today must keep working"* — is a cost we take on **at 1.0**, not now. That freedom is a reason to get the grammar right while revising it is cheap, not a licence to be careless; it simply means we optimise the additions on their merits rather than around a compatibility surface that is not yet frozen.

## Decision

We extend the expression grammar along four axes. The additions are naturally additive — nothing existing changes meaning — but that is a happy property, not the driver: pre-1.0 we are free to revise the grammar without a compatibility obligation, so each choice below is made on its own merits.

1. **Math functions** — add `cos`, `sqrt`, `pow(base, exp)`, `smoothstep(edge0, edge1, x)`, and `mod(a, b)`. `mod` is **floored** (`a - b*floor(a/b)`, divisor-signed) so it wraps cleanly for cyclic hue/time. All are total: `sqrt`/`pow` of an out-of-domain input yield `NaN`/`inf`, never a panic; `smoothstep` with `edge0 == edge1` degenerates to `0` through the existing `max().min()` clamp.
2. **Constants** — add `pi` and `tau` as bare identifiers resolving to literals (checked before the variable lookup).
3. **Branching** — add the six comparison operators (`>`, `<`, `>=`, `<=`, `==`, `!=`) at a **new lowest-precedence tier**, each yielding `1.0`/`0.0`, and a `select(cond, x, y)` conditional that returns `x` when `cond != 0.0` else `y`, **evaluating only the taken branch**. We deliberately add **no** boolean operators: with clean `0/1` comparison results, `min`/`max`/`1 - c` already express and/or/not.
4. **Two new variables** — plumb the two already-computed `AnalysisFrame` fields that expressions can't yet read: `bpm` as **`tempo`** (the grammar's 8th variable) and `novelty` as **`novelty`** (the 9th, the experimental spectral track-change signal — ~0 within a steady segment, spiking at a boundary). No new DSP; both are already deterministic (`tempo` from hop-clock autocorrelation, `novelty` from the Plan 0009 spectral-flux detector). `tempo`'s scale (0 until warm, then ~60–200) pairs naturally with the new comparisons (`select(tempo > 128, aggressive, calm)`); `novelty` is a transient authors gate scene-change accents on. `novelty` is documented as **experimental** (its shape may change), which is cheap to promise pre-1.0.

Separately, **an unknown parameter name becomes a surfaced load-time *warning*, not a rejection.** The preset loads and applies its known bindings; the unknown ones are collected into the load report alongside the existing hard errors. This requires each built-in system to **declare its parameter vocabulary** so the loader can check membership at load (once), never per frame.

We rejected a **ternary** `cond ? x : y` in favor of `select()` (the function reuses the existing call/arity machinery with no new tokens or precedence tier, and skipping the untaken branch avoids `NaN` poisoning that a `lerp`-based blend cannot); we rejected **hard-rejecting** unknown params (it discards an otherwise-good preset over one typo, against NFR §10's degrade-never-crash); and we rejected **embedding a general expression crate** (it forfeits the size cap and the by-construction total/panic-free guarantee the hot path needs).

## Consequences

### Positive
- Closes the concrete authoring walls the `preset-author` lane hit: real `cos`, easing via `smoothstep`, response-shaping via `pow`, cyclic wrap via `mod`, tempo-aware and threshold-gated looks via `tempo` + comparisons + `select`.
- Incidentally non-breaking: every addition was previously a compile error (`>`/`<` were unknown chars; `cos`/`pi`/`select`/`tempo` were `UnknownIdent`), so no preset in the shipped set uses these names — nothing existing changes meaning. (This is a convenience, not a requirement — pre-1.0 we could break the format freely; we simply didn't need to.)
- The typo footgun stops being silent without becoming brittle: a mistyped param is reported, the rest of the preset still runs.
- Stays pure-Rust, allocation-free, and total — the hot-path pragma on `expr.rs` holds; determinism (NFR §6) is preserved (every new op is a pure function of the input window; `tempo` is already deterministic).

### Negative — the price
- **The grammar surface roughly doubles**, so there is more to keep stable **once 1.0 freezes the format** — comparison semantics, `select` truthiness (`!= 0.0`), and floored-`mod` sign become commitments then. Until 1.0 they remain revisable, so the cost is deferred, not avoided (ADR-0002's flagged price lands at 1.0).
- **`tempo` is an authoring gotcha**: `0` until the tracker warms and an unbounded `60–200` scale, unlike the `~0–1` bands. It needs a prominent doc note (scale it, or use it under a comparison).
- **A small drift risk**: each system's declared param list sits beside its `set_param` match; the two can fall out of sync. Mitigated by a per-system test asserting the declared names are exactly the handled ones (see the plan), plus a sync comment.
- The tokenizer grows two-character lookahead (`>=`, `<=`, `==`, `!=`) and a bare `!`/`=` becomes an explicit error — a slightly larger lexer.

### Neutral
- **The C ABI is untouched.** Preset evaluation is entirely core-internal; grammar, variables, and typo handling never cross the `extern "C"` seam. `LMV_ABI_VERSION` does not move.
- **`novelty` is exposed as an experimental variable.** It is the last already-computed `AnalysisFrame` field expressions couldn't read. It is native-only across the C ABI (no query function), but preset evaluation is core-internal, so it is available on both frontends regardless. It is labelled experimental in the docs (its shape may change or it may be withdrawn), which is a safe promise pre-1.0 — the freedom to revise the grammar is exactly why exposing it now is low-risk.
- No new dependency; no new crate.

## Alternatives considered

### Ternary `cond ? x : y` instead of `select(cond, x, y)`
Familiar C-style syntax, but it costs two new tokens (`?`, `:`), a new right-associative precedence tier, and more parser surface — for no capability `select()` lacks. `select()` slots into the existing `Func`/arity/`Call` machinery with zero grammar-shape change, and because it evaluates only the taken branch it can guard `NaN`/`inf` (`select(x >= 0, sqrt(x), 0)`), which a `lerp(y, x, c)` blend cannot (the untaken `sqrt` of a negative still poisons the result). Rejected.

### Hard-reject a preset with an unknown parameter name
Consistent with unknown *system*/*function*/*variable* already being load errors, and it surfaces typos loudest. But it throws away every good binding in the file over a single typo, which contradicts NFR §10 (a misbehaving preset must degrade, not vanish) — and a live show is exactly when you don't want a one-character slip to blank a scene. Warn-but-load surfaces the mistake while keeping the working bindings. Rejected.

### Dedicated boolean operators (`&&`, `||`, `!`)
Once comparisons yield clean `0.0`/`1.0`, `min(a, b)` is and, `max(a, b)` is or, and `1 - c` is not — the operators add grammar for no new expressive power. Rejected in favor of documenting the `min`/`max` idiom.

### A `%` operator for modulo
Would need a new token and a precedence decision. The `mod(a, b)` function form matches the other additions, avoids lexer/precedence churn, and lets us pin the floored (divisor-signed) semantics authors actually want for wrapping. Rejected in favor of the function.

### Embed a general expression-evaluator crate (evalexpr / meval / rhai-as-expressions)
Would hand us comparisons, functions, and constants "for free," but contradicts the lightweight/size NFR (§4) and ADR-0002's pure-Rust minimal-surface stance, and — decisively — a general evaluator does not guarantee the *total, allocation-free, panic-free* property the per-frame hot path requires (the panic-denial pragma on `expr.rs`). The hand-rolled evaluator keeps that guarantee by construction. Rejected.

### Freeze the grammar and defer all expansion
The interview's minimal option: fix only `tempo`, the footgun, and the stale docs. Rejected by the user — the `preset-author` lane is hitting the `cos`/easing/threshold walls *now*, so the expansion has a live consumer rather than being speculative.

## Notes

- This ADR flips to **accepted** when Plan 0019 closes (per the ADR-0005 close ceremony).
- The stale `docs/presets.md` (claims 10 presets / 2 systems; the code ships 17 / 5) is corrected as the final phase of Plan 0019, describing the finished grammar in one rewrite rather than twice.
