# The second duty: feeding the API's evolution (and the curation handoff)

The preset surface is small and **deliberately growing**. This lane is the engine's best source of
grounded signal about *what to grow next*, because you hit the walls first, with a concrete look in
hand. Consuming the API is half the job; **reporting where it stopped you is the other half.**

## Mindset: friction is signal, not a dead end

When you reach for something that doesn't exist — a `cos`, a `tempo` variable, an easing so a value
stops snapping, a curve family, a whole scene idiom — the instinct is to work around it and move on.
**Don't just work around it silently.** Note it. A workaround (`sin(x + 1.5708)` for cosine, a
hand-tuned `lerp` chain faking a smooth) is a marker of a missing capability, and the person who felt
the friction is the right person to report it.

This does **not** mean you stop authoring. You still deliver the best preset the *current* surface
allows. You just also carry out what you learned about the surface's edges.

## How to capture and route it

Keep a running list while you work (a couple of lines in the scratchpad is enough). At the end of a
session, if the friction is real and recurring, hand `architect` a short note — `architect` owns the
decision of whether it becomes an ADR + a `dev` plan. You do **not** write the ADR or the code; you
supply the grounded motivation.

A good feedback note is concrete and shows the look you couldn't reach:

```
API feedback — preset-author, <date>

Wanted: <the look/behavior you were going for, in one line>
Reached for: <the capability — e.g. "a cos() function", "per-param easing", "a 5-fold star tiling">
Current surface can't: <why — what you had to do instead, or that it's simply absent>
Concrete example:
  <the binding or preset snippet where it bit>
Impact: <how often this comes up / how much it limits looks — one line>
Not: engine design. That's architect's call — this is the motivating friction.
```

Route it: "This is engine work, not a preset. Handing architect a feedback note; start a fresh
`/architect` session to decide if it's ADR-worthy." Then stop reaching into Rust.

**Before filing, check it isn't already planned** (below) — if it is, say so ("this aligns with
Plan 0018 / ADR-0019") rather than filing a duplicate. That's still useful signal: it tells architect
the planned work has real demand.

## Current known gaps (as of 2026-07-23 — shrinking; re-verify)

> These are the walls the surface has *today*. As the app develops they get filled — confirm a gap is
> still real (grammar/scene source of truth) before reporting it.

**Expression grammar:**
- No `cos` (spell as `sin(x + 1.5708)`), no `sqrt pow exp log mod smoothstep noise` — the set is the 7
  in `grammar.md`.
- No constants (`pi`/`tau`) — literals only.
- No comparisons/logic/ternary — you cannot express "if beat then A else B" directly (only arithmetic
  tricks like `floor(2.99 * beat)`).
- No `tempo`/`bpm` variable, no per-bin spectrum access, only the 3 bands + `onset`/`beat`/`bar`/`time`.
- No stateful expressions (no `smooth()`/`slew()`) — the evaluator is pure by hard invariant; smoothing
  can only live in the render layer (that's what ADR-0019 proposes).

**Scenes / vocabulary:**
- Only 5 scenes. No feedback/reaction-diffusion scene (designed, Plan 0014/ADR-0012), no GPU-compute
  particles (Plan 0016/ADR-0015), no 3D, no boids/walkers as scenes. A preset cannot invent a system.
- Only one curve family (`maurer_rose`), four star tilings (4/6/8/12) — more are engine work.
- No author-supplied shader/WGSL pass, no custom color function/palette — color is the one shared
  cosine palette via `hue`.

**View / compositing (all pending ADR-0018, not landed):**
- No view zoom/pan (only per-shape `scale`), no background behind scenes (each clears), no trails, no
  mirror/kaleidoscope, no multi-scene compositing, no effect reordering.

**Parameter behavior:**
- No easing/smoothing — band values are noisy, beat values snap (pending ADR-0019).
- No structural crossfade — discrete params (`variant`, `visible_depth`) snap between cached states.

**Determinism caveat (constrains what a preset can promise):** feedback sims and (future) chaotic
attractors are not bit-identical across GPU vendors — "identical on every device" holds *visually*,
not pixel-exactly. Don't author a preset that depends on exact cross-machine pixels.

## The horizon — what's already planned (don't file duplicates)

These are designed/approved but **not landed** — a preset cannot use them yet, but they're coming, so
align feedback rather than re-proposing:

- **ADR-0018 — engine-wide compositing** (proposed; Plan 0018 approved). Adds preset-expressible
  view `zoom`/`pan_x`/`pan_y`, gradient/vignette background (`bg_hue`/`bg_bright`/`bg_vignette`),
  feedback `trails`, a geometry mirror (`mirror_order`/`mirror_reflect`) and a screen-space
  kaleidoscope (`kaleido_order`/`kaleido_angle`). Fixed composite order (not a general graph).
- **ADR-0019 — eased parameters** (proposed; Plan 0018 phase). Adds an optional `[smoothing]` table
  (`param = seconds` time constant) so band/beat params stop snapping. Expression layer stays pure.
- **Plan 0016 / ADR-0015 — GPU compute particles** (approved; proposed). A new particle scene family
  (strange attractors first) with params `a b c d size hue fade reseed count` and a `[particles]`
  config table.

When any of these land, the grammar/scene/`shot` source-of-truth files change — this skill's snapshots
go stale and should be refreshed (that refresh is itself preset-lane maintenance).

## NFR limits a preset must respect

From `docs/nfr.md` — a preset that violates these is a bug, and pushing past them is engine work:

- **60 fps @ 1080p on an integrated GPU** is the floor. Dense geometry (`samples`, `max_depth`,
  `visible_depth`) and heavy additive overdraw (swarm `size`×density) are the levers that blow it.
  `MAX_SEGMENTS = 20_000` caps line geometry (overflow is surfaced, not silent).
- **~10 MB size cap; no new dependency** for a preset feature — if a look needs a new crate, that's an
  architect/ADR event, never a preset.
- **Determinism / seeded randomness** (NFR §6) — there is no unseeded randomness in the expression
  grammar anyway; don't assume any.
- **Flat memory over a 4-hour session** (NFR §12) — relevant only if a preset drives a new stateful
  scene, which is engine territory.

## Curation handoff — flagging a preset to ship

Your default output is a **user-directory preset**. When one is strong enough to ship in the curated
set, **flag it — do not embed it yourself.** Embedding touches Rust in two coupled spots, which is a
`dev` task (ADR-0017):

1. `presets/<name>.toml` — the on-disk file (this part you can author).
2. `core/src/preset/mod.rs` — add a `(file, include_str!)` entry to the `EMBEDDED` array **and bump its
   length type** `[(&str, &str); N]`.
3. `core/tests/preset.rs` — bump the `assert_eq!(presets.len(), N)` count assert.

Hand off like: "Preset `<name>` is a strong ship candidate. Embedding is a `dev` change — it needs the
`EMBEDDED` array + length bump in `core/src/preset/mod.rs` and the count assert in
`core/tests/preset.rs`. Start a `/dev` session to embed it." You propose; `dev` embeds; the fresh-lane
boundary holds.
