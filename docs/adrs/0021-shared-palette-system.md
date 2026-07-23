# ADR-0021 — Shared preset-controllable palette system: baked gradient LUT, named + custom stops, bindable modulation

> **Status:** proposed
> **Date:** 2026-07-23
> **Related plan(s):** [0020](../plans/0020-shared-palette-system.md)
> **Supplements:** [ADR-0002](0002-layered-preset-architecture.md) (adds a color axis to the layer-1/2 preset surface)

## Context

Every built-in scene colors itself through the **same hardcoded iq cosine palette**, duplicated
in two places: `fragment_field.rs` (in the WGSL `palette()` fn) and `swarm.rs` (a CPU `palette()`
fn), both with `d = (0.10, 0.42, 0.62)`. A preset's only color lever is a scalar `hue` offset
that rotates that one rainbow. This surfaced as concrete `preset-author`-lane friction (commit
`76a2fb4`, while making the fragment and swarm presets distinct) in two ways that no preset can
work around:

- **Fragment fields cannot hold a cohesive mood.** Displayed color is `palette(field*0.6 + hue)`
  with the `0.6` span hardcoded, so every field smears ~60% of the same wheel; an all-warm ember
  or all-cool glacier is reachable only by accident (low zoom narrowing the field's spatial range).
- **Swarm color is completely unreachable.** Each particle's hue is `rng.next_f32()` across the
  *full* wheel, and color is `palette(p.hue + self.hue)`, so the `hue` param only rotates an
  already-full rainbow — a visual no-op. Swarm presets cannot differ in color at all.

None of the approved horizon addresses this: ADR-0018 adds a *background* color (`bg_hue`), not a
*scene* palette; ADR-0020/Plan 0019 widens the expression grammar, not the color model; Plan 0016
adds a new scene. Color is a genuine missing axis of the preset surface, and — because the same
palette is duplicated per scene and every future scene will re-duplicate it — the fix belongs in
one shared place, not bolted onto two scenes. The forces: it must stay **allocation-free and
panic-free per frame** (hot path §5), **deterministic** (NFR §6 — no clock/unseeded randomness in
color), **lightweight** (no new dependency), and it must **not regress the 17 shipped presets**
(their current look is the baseline).

The interview fixed four directions: support **both** curated named palettes **and** low-level
custom gradient stops; make color **fully audio-bindable**, not a fixed per-preset choice; land it
as **one shared module** every scene uses; sequence it **independently** (core-only, C ABI frozen).

## Decision

We will add a shared palette module, `core/src/render/palette.rs`, that every scene colors through
instead of a private hardcoded function. The model has three parts:

**1. A palette is a gradient, baked once at load into a 256-entry RGB lookup table (LUT).** A
preset declares its gradient in an optional `[palette]` config table (ADR-0007-style structural
config, validated at the load boundary) as **either** a built-in `name` (a curated set — `spectrum`
[the current iq cosine, the default so shipped presets are unchanged], `ember`, `ice`, `mono`,
`aurora`, …) **or** a list of custom `stops` (`at` position + hex/`rgb` color). Named palettes are
themselves defined as built-in stop lists (some generated from the cosine model), so named and
custom share one representation. Baking is a pure function of the config — no clock, no randomness —
so it is deterministic; it happens off the hot path, only on preset load.

**2. The baked LUT is the single source of truth both the GPU and CPU sample.** The fragment scene
receives the LUT as a **256×1 1D texture** (linear-filtered) added to its bind group; the swarm
samples the identical Rust-side `[Rgb; 256]` array on the CPU for its per-particle color. One bake,
two consumers, no drift.

**3. Color is fully bindable through named layer-2 params (ADR-0002), not the static config.** The
gradient *shape* is config (baking arbitrary gradients per frame is wasteful and non-deterministic),
but everything that *modulates* it is a normal bindable expression over the audio vocabulary:

- **Shared:** `saturation` (scales chroma around luma), `hue` (rotates the LUT sample coordinate).
- **Fragment:** `color_span` (replaces the hardcoded `field*0.6` — low span = cohesive mood) and
  `color_center` (where the field's window sits in the gradient).
- **Swarm:** `hue_spread` and `hue_center` (particle hues occupy `center + (particle_hue-0.5)*spread`
  — `spread=1` reproduces today's full wheel, `spread=0` is a single color).

**Bindable palette *selection*** (the "fully bindable" requirement) is met by letting a preset
declare a second palette `[palette_b]` and bind a `palette_mix` expression (`0..1`) that crossfades
A↔B per frame — the scene samples both LUTs and lerps. A beat can push the mix smoothly, with no
flicker. Delivered as one shared load-time seam (a new optional `Scene::set_palette(&Palette)`
method, the second and last thin off-hot-path widening of the trait after ADR-0007's `configure`)
plus the existing per-frame `set_param` path — no new mechanism, no C ABI change.

## Consequences

### Positive
- Both reported gaps close as **first-class, bindable knobs**: a cohesive-mood field (`color_span`
  low + a warm palette), a coherently-colored swarm (`hue_spread` < 1), and audio-driven palette
  crossfades — none reachable before.
- **One palette abstraction** every current and future scene inherits (OCP/DIP): the reaction-
  diffusion scene (ADR-0012), the compute-particle scene (Plan 0016), and later scenes get color
  control for free instead of re-duplicating a `palette()` fn.
- Named + custom unified under one baked-LUT primitive keeps the authoring surface small while the
  ceiling (arbitrary gradients) is high.
- **No visual regression:** `spectrum` (the default when no `[palette]` is given) is the exact
  current cosine, so the 17 shipped presets render unchanged until re-authored.
- Deterministic and lightweight: bake is pure and off-hot-path; sampling is alloc-free; no new crate.

### Negative (the price we pay)
- **A third preset compatibility surface.** After the expression grammar and (future) Rhai host API,
  the palette schema (`[palette]` fields, built-in palette names, the color param names) is now a
  contract presets depend on. Pre-1.0 we owe no back-compat (the app is 0.4.0), but post-1.0 it
  joins the versioned surfaces.
- **The `Scene` trait widens once more.** ADR-0002 keeps it thin; this adds a second optional
  method (`set_palette`). Justified and minimal, but the trait is now two methods past its ADR-0002
  shape — a third widening should prompt asking whether the seam is still the right one.
- **The fragment scene's bind group grows** from one uniform to uniform + 1D texture + sampler (two
  textures with A/B). Trivial cost, but every scene that wants palette color takes on a texture bind.
- **Color is visually, not bitwise, identical across GPU vendors** — hardware linear filtering of
  the LUT texture differs subtly by adapter (the same caveat ADR-0018 records for feedback). Fine
  for a display value; a preset must not depend on exact cross-machine pixels.

### Neutral
- The gradient shape stays load-time config while its modulation is per-frame — a deliberate split
  that mirrors ADR-0007's `[generator]` (structure at load) vs. named params (motion per frame).

## Alternatives considered

### Alternative A — Uniform stop-array evaluated in-shader (no LUT texture)
Pass ≤N stops as a uniform array and interpolate in WGSL each pixel. Avoids a texture+sampler, but
caps stop complexity, adds a per-pixel loop to the fragment shader, and forces the interpolation to
be written **twice** (WGSL for fragment, Rust for the swarm's CPU color) — two sources of truth that
drift. Rejected: the baked LUT is one source both consumers sample, with unbounded stop complexity
and free hardware interpolation.

### Alternative B — Expose the cosine coefficients (a/b/c/d) as the primary model
Make the iq palette's four `vec3` phase vectors bindable params. Compact and fully in-shader, but
the authoring bar is "tune phase vectors," not "pick colors," and it structurally cannot express an
arbitrary gradient (the user explicitly wants custom stops). Rejected as the *authoring* model; the
cosine generator survives internally to define some built-in named palettes.

### Alternative C — Minimal per-scene fix, no shared module
Just add swarm `hue_spread` and one fragment palette knob in place. Smallest diff, but the next
scene re-duplicates the palette again — exactly the duplication that caused this ADR. Rejected; the
interview chose the shared module.

### Alternative D — Bindable integer palette index (select 1 of N palettes per frame)
Let an expression pick palette #k each frame. The float→int selection flickers and aliases under
audio noise, and switching is a hard cut. Rejected in favor of the A/B `palette_mix` crossfade,
which gives smooth, bindable selection between two configured palettes.

### Alternative E — Full perceptual color management (OKLab working space, gamma-correct blends)
Interpolate gradients and crossfades in OKLab for perceptually even ramps. Correct but heavier than
v1 needs and orthogonal to the two gaps. Deferred: the LUT bake is the natural place to add OKLab
interpolation later without changing the preset surface — noted as a followup, not smuggled in now.

## Notes

Motivating friction: `preset-author` feedback note (2026-07-23), commit `76a2fb4`. The palette
duplication is at `core/src/render/scenes/fragment_field.rs` (`palette()` in the WGSL `SHADER`) and
`core/src/render/scenes/swarm.rs:262`. This ADR fixes the color model only; it does not add color
management (Alt E), does not touch the C ABI, and leaves preset **re-authoring** (exploiting the new
palettes) to the `preset-author` lane as a followup.
