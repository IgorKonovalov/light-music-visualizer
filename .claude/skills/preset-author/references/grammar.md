# Preset grammar — complete reference

> **Snapshot: 2026-07-23.** The expression language and schema are engine code and **change as the
> app develops**. Before relying on any list here, confirm it against the source of truth:
> - variables → `VAR_NAMES` in `core/src/preset/expr.rs`
> - functions + arity → `Func::from_name` / `Func::arity` in `core/src/preset/expr.rs`
> - top-level schema → `RawPreset` in `core/src/preset/schema.rs`
> - structural config → `into_config` / `into_lsystem` / `into_star` in `core/src/preset/schema.rs`
>
> If code and this file disagree, the code is right and this file is stale — fix it, and if the delta
> is a capability you wanted, log it as API feedback (`api-feedback.md`).

## Top-level file schema

Deserialized by `RawPreset` (`core/src/preset/schema.rs`):

| Key | TOML type | Required | Notes |
|-----|-----------|----------|-------|
| `system` | string | **yes** | Must match `SystemKind::from_name`. Missing/unknown ⇒ load error. |
| `name` | string | no | Defaults to the `system` string. This is what `--preset` matches. |
| `[params]` | table string→string | no | Each key = a scene param name; each value = an **expression string**. |
| `[curve]` | table | no | Only for `parametric_curve`. |
| `[generator]` | table | no | **Required** for `lsystem` and `star_pattern`. |

There is **no** `[meta]`, `[palette]`, `[color]`, `[easing]`, or `[smoothing]` table today. Color is
driven only through the `hue` param into a shared cosine palette. `[smoothing]` is proposed
(ADR-0019) but **not implemented** — a preset that includes it has those keys silently ignored, so do
not author it as if it works.

**Every `[params]` value must be a quoted string.** `warp = 0.4` is a TOML type error; write
`warp = "0.4"`. Bindings are applied name-sorted (a `BTreeMap`), so file order is irrelevant.

## Expression grammar

Standard-precedence recursive descent; compiled once at load, evaluated per-param per-frame; total,
panic-free, allocation-free.

```
expr   := term  (('+' | '-') term)*
term   := unary (('*' | '/') unary)*
unary  := ('-' | '+')? primary
primary:= number | ident | ident '(' expr (',' expr)* ')' | '(' expr ')'
```

### Variables — exactly 7

`bass mid treb onset beat bar time`

| Var | Meaning | Range / shape |
|-----|---------|---------------|
| `bass` | bass-band magnitude (~20–250 Hz) | raw, **small**; full-scale sine ≈ 1.0, real material far lower |
| `mid` | mid-band magnitude (~250–4000 Hz) | same small scale |
| `treb` | treble-band magnitude (~4–18 kHz) | same; reads smallest of the three |
| `onset` | spectral-flux attack strength | a transient spike, not a level |
| `beat` | beat gate | `0.0` or `1.0` (a `bool` coerced) |
| `bar` | beat phase | sawtooth `[0, 1)` — 0 on each beat, ramps to the next |
| `time` | scene clock | seconds, monotonic, unbounded |

Spelled exactly this way — `mid` not `mids`, `treb` not `treble`. **There is no `tempo`, no `bpm`, no
`mid_high`, no per-bin access.** Wanting any of those is API feedback.

### Operators

Binary `+ - * /`; unary prefix `-` (negate) and `+` (no-op); parentheses for grouping; `,` separates
function arguments. **Nothing else** — no `< > == != && || !`, no ternary/`if`, no `%`, no `^`.
Division by zero yields `inf`/`NaN` (not a panic), which becomes broken geometry downstream — avoid
denominators that can reach zero.

### Functions — exactly 7

| Call | Arity | Semantics |
|------|-------|-----------|
| `sin(x)` | 1 | `x.sin()`, radians |
| `abs(x)` | 1 | `x.abs()` |
| `floor(x)` | 1 | `x.floor()` — the tool for discrete selection (`variant`, `visible_depth`) |
| `min(a, b)` | 2 | lesser |
| `max(a, b)` | 2 | greater |
| `clamp(x, lo, hi)` | 3 | `x.max(lo).min(hi)` — total even if `lo > hi` |
| `lerp(a, b, t)` | 3 | `a + (b - a) * t` — **`t` is not clamped** |

That is the entire set. **No `cos`** → use `sin(x + 1.5708)`. No `sqrt pow exp log mod smoothstep
noise mix step fract sign round ceil`. **No constants** — no `pi`/`tau`/`e`; write `3.14159`,
`6.28318` literally. Each missing function you reach for is worth noting as feedback.

### Output range

An expression returns a raw `f32` with **no evaluator-side clamping** — it is written straight into
the scene param. Each scene decides its own sane range; **you clamp yourself.** The universal idiom:

```
gain-then-bound:  clamp(bass * 14, 0, 1.8)
```

Raw bands read small, so a bare `bass` barely moves the look and an un-gained `bass * 40` blows out.
Gain to taste, then `clamp` to the param's sane window (see `systems.md` for each param's window).

## `[curve]` / `[generator]` structural config

Static data the generator consumes **once at load** (not expressions, not per-frame). Validated at
load; a bad value fails the whole preset with a surfaced error.

### `[curve]` — for `parametric_curve`

| Key | Type | Required | Allowed |
|-----|------|----------|---------|
| `family` | string | yes (if table present) | **`"maurer_rose"`** only — anything else is rejected |

Absent `[curve]` ⇒ family default. Only one curve family exists today; a second one is engine work.

### `[generator]` as L-system — for `lsystem` (required)

| Key | Type | Required | Default | Rule |
|-----|------|----------|---------|------|
| `axiom` | string | **yes** | — | non-empty |
| `rules` | table char→string | **yes, ≥1** | — | each key is a **single character** |
| `angle_deg` | float | no | `25.0` | finite; turn angle for `+`/`-` |
| `max_depth` | integer | no | `4` | `1..=7` (`MAX_LSYSTEM_DEPTH`) |
| `seed` | integer | no | `0` | reserved; deterministic today |

Turtle vocabulary in the expanded string: `F`/`G` draw forward, `f` moves without drawing, `+`/`-`
turn by `angle_deg`, `[`/`]` push/pop branch state, any other char is an inert grammar variable.
Example: `rules = { X = "F+[[X]-X]-F[-FX]+X", F = "FF" }`.

### `[generator]` as star pattern — for `star_pattern` (required)

| Key | Type | Required | Default | Allowed |
|-----|------|----------|---------|---------|
| `tiling` | string | **yes** | — | `square`/`4`/`4.4.4.4`→4, `hexagon`/`6`/`6.6.6`→6, `octagon`/`8`/`4.8.8`→8, `dodecagon`/`12`/`3.12.12`→12 |
| `contact_angle_deg` | float | no | `30.0` | finite; the Hankin contact angle |

Only these four tilings exist; a fifth order is engine work.

**Geometry cap:** all line scenes share `MAX_SEGMENTS = 20_000`. Overruns are truncated and surfaced
via `CapOverflow` at load (never silently cut) — if you hit it, the preset is too dense (lower
`samples`, `max_depth`, or `visible_depth`).

## Validation & error surface

All load failures are recoverable `PresetError`s — the engine keeps the last good preset and never
crashes (NFR §10). Knowing the surface tells you what you must get right:

| Error | Trigger |
|-------|---------|
| `Toml(..)` | malformed TOML, wrong value type (e.g. bare number for a param), missing `system` |
| `UnknownSystem(s)` | `system` not in `SystemKind::from_name` |
| `Expr { param, err }` | a param expression fails to compile (see below) |
| `Config(msg)` | bad `[curve]`/`[generator]` — unknown family/tiling, empty axiom, multi-char rule key, no rules, out-of-range `max_depth`, non-finite angle, missing required table |
| `Io(msg)` | file unreadable (directory-load path only) |

Expression compile errors (`ExprError`): `UnexpectedChar` (illegal char like `@`), `BadNumber`,
`UnknownIdent` ("unknown variable or function '…'" — this is what a typo'd function or a nonexistent
variable produces), `WrongArity`, `UnexpectedToken`, `UnexpectedEnd`, `TrailingTokens` (leftover
tokens, e.g. `1 2`).

**What is NOT validated — the footgun:** an **unknown param name is silently ignored**. Every scene's
`set_param` ends in `_ => {}`, so a misspelled param (`thicknes`, `col`) compiles clean and does
nothing. The grammar can't catch this — *you* catch it by verifying param names against the scene's
`set_param` before you author (SKILL.md step 2). There is also no range/NaN checking on expression
output.
