# Preset authoring

Presets are TOML files (ADR-0002). A preset names a built-in **system** and binds
that system's **named parameters** to **expression strings** over the audio
analysis. Line-art systems (ADR-0007) additionally take a declarative
**structural-config table** (`[curve]` / `[generator]`) that is *not* expressions.

Files here are the curated set embedded into the binary and seeded into the
per-user preset directory on first run (Plan 0007). Editing a file and dropping
it in that directory hot-reloads it.

## Skeleton

```toml
system = "parametric_curve"   # which built-in system (see the table below)
name   = "My Rose"            # optional; defaults to the system name

[curve]                       # or [generator] — structural config (line systems)
family = "maurer_rose"

[params]                      # named parameter -> expression string
n     = "6"
scale = "0.7 + bass * 0.4"
hue   = "0.5 + time * 0.02 + treb * 0.3"
```

## The expression language

Each `[params]` value is a pure expression evaluated every frame. A malformed
expression (or structural config) makes the whole preset fail to load with a
surfaced error — the engine keeps the last good preset, never crashes (NFR 10).

- **Variables:** `bass mid treb onset beat bar time`
  (`beat` is 0/1; `bar` is the 0..1 beat phase; `time` is seconds).
- **Functions:** `sin abs floor min max clamp lerp`.

## Systems and their named parameters

| System            | Named `[params]`                                                         |
|-------------------|--------------------------------------------------------------------------|
| `fragment_field`  | `warp` `hue` `zoom` `glow` `flash` · `pan_x` `pan_y`                      |
| `swarm`           | `force` `spin` `burst` `hue` `brightness` `size` · `zoom` `pan_x` `pan_y` |
| `parametric_curve`| `n` `d` `samples` `thickness` `hue` `spin` `scale` `brightness` `draw_progress` · `zoom` `pan_x` `pan_y` `mirror_order` `mirror_reflect` |
| `lsystem`         | `visible_depth` `rotation` `hue` `draw_progress` `thickness` `scale` `brightness` · `zoom` `pan_x` `pan_y` `mirror_order` `mirror_reflect` |
| `star_pattern`    | `variant` `rotation` `hue` `draw_progress` `thickness` `scale` `brightness` · `zoom` `pan_x` `pan_y` `mirror_order` `mirror_reflect` |
| `reaction_diffusion` | `feed` `kill` `flow` `inject` `hue` `contour` `hatch` `glow`           |
| `attractor`       | `a` `b` `c` `d` `size` `hue` `fade` `reseed`                              |

Unbound parameters fall back to each system's defaults. Unknown parameter names
are ignored. The params after the `·` are the shared **view transform** and
line-**mirror** controls (Plan 0018) — see [Engine-wide controls](#engine-wide-controls-plan-0018).
Every system additionally accepts the engine-stage params `bg_*`, `trails`, and
`kaleido_*` documented there.

### Line-art parameter notes (Plan 0010)

- `thickness` — stroke weight (roughly 1–5); scaled to a projector-friendly glow.
- `hue` — offset into the shared cosine palette (add `time * k` for a slow drift).
- `scale` — overall size in the frame; `draw_progress` in `0..1` reveals the
  figure from the start (a line-draw-on; ride it on `bar` for a per-beat redraw).
- `parametric_curve`: `n`/`d` are the rose parameters, `spin` is angular velocity
  (rotation = `spin * time`), `samples` the chord count (clamped to the segment
  cap).
- `lsystem`: `visible_depth` picks which precomputed iteration is shown — drive
  it off a band/beat to *grow* the structure (e.g. `4 + floor(2 * bass)`);
  `rotation` is an angle in radians, so multiply by `time` yourself.
- `star_pattern`: `variant` selects one of the precomputed contact-angle variants
  (0..2, clamped) — swap it on a beat for a structural accent
  (e.g. `floor(2.99 * beat)`); `rotation` is an angle in radians.

## Engine-wide controls (Plan 0018)

These sit in the **render composite** (ADR-0018), not in one scene, so they are
audio-bindable like any other param. All default to *off/identity*, so a preset
that binds none renders exactly as before.

### Shared view transform — `zoom`, `pan_x`, `pan_y`

A camera zoom about the frame centre, then a pan. Applied by `fragment_field`,
`swarm`, and the three line systems (`parametric_curve` / `lsystem` /
`star_pattern`).

- On the **line** and **swarm** scenes, `zoom > 1` moves the camera *in* (geometry
  bigger); `zoom = 1` is no zoom. `pan_*` shift in world units. Try
  `zoom = "1 + bass * 0.6"` for a kick-driven pump.
- On the **fragment field**, `zoom` is the pre-existing field-density knob (it
  scales the *sample* coordinates, so a *higher* `zoom` shows *more* field cycles —
  the opposite sense to the line scenes, kept for the shipped fragment presets);
  `pan_x` / `pan_y` slide the sampled field window.
- `reaction_diffusion` and `attractor` are full-screen and ignore the view
  transform.

### Background pass — `bg_hue`, `bg_bright`, `bg_vignette`

An audio-tintable gradient + vignette backdrop drawn *before* the scene, engine-
wide. `bg_bright = 0` (the default) is a black backdrop; raise it to reveal the
gradient. `bg_hue` offsets into the shared cosine palette; `bg_vignette` (0..1)
darkens the corners. Visible behind the **sparse** scenes (lines, swarm,
attractor) where the gaps show through; the full-screen scenes (fragment,
reaction-diffusion) draw over it.

### Geometry mirror (line systems) — `mirror_order`, `mirror_reflect`

Replicates a line scene's segments under N-fold rotational symmetry to build a
true geometric fractal. `mirror_order` is the fold count (rounds, clamped to
`1..=24`; `1` = no mirror). `mirror_reflect >= 0.5` adds a reflected copy per
sector (dihedral). Distinct from the screen-space `kaleido_*` below: this folds
the *geometry*, that folds the finished *pixels*. High order on a dense curve is
capped at the segment limit and the drop is surfaced at load-time-style — never a
silent cut.

### Feedback trails — `trails`

Routes the composited frame through a fade-and-accumulate feedback (max-decay), so
moving shapes leave light trails. `trails = 0` (default) is off; `0 < trails < 1`
sets the per-frame decay (higher = longer trails). Best on a scene with real
motion (a spinning curve, a drifting swarm).

### Screen-space kaleidoscope — `kaleido_order`, `kaleido_angle`

Folds the finished frame into `kaleido_order` mirrored wedges before present.
`kaleido_order < 2` (default) is a passthrough; `>= 2` folds (clamped to 48).
`kaleido_angle` (radians) rotates the fold — ride it on `time` for a turning
kaleidoscope. Works on any scene.

## Eased parameters — the `[smoothing]` table

An optional top-level `[smoothing]` table low-passes chosen params so band- and
beat-driven motion eases instead of snapping (ADR-0019). Each entry is a **time
constant in seconds** (a bare number, *not* an expression):

```toml
[smoothing]
zoom  = 0.12   # a punchy pump that still eases
bg_bright = 0.3
hue   = 0.4    # a slow, fluid hue drift
```

A param not listed is applied instantly (today's behaviour); `0` also means no
smoothing. The smoothing runs on real elapsed time, so it is identical at any
refresh rate, and it resets on a preset switch (a switch snaps to the new preset's
first value). Validated non-negative and finite at load.

## Structural config (line systems only)

Declarative data the generator/sampler consumes once at load — **not**
expressions. Validated at load; a bad value is a surfaced error.

### `[curve]` — for `parametric_curve`

| Key      | Values           | Notes                          |
|----------|------------------|--------------------------------|
| `family` | `maurer_rose`    | The curve family. Required.    |

### `[generator]` — for `lsystem`

| Key         | Type            | Notes                                                       |
|-------------|-----------------|-------------------------------------------------------------|
| `axiom`     | string          | Starting string. Required, non-empty.                       |
| `rules`     | table `k = "v"` | Each key a single character (the predecessor). Required.    |
| `angle_deg` | number          | Turn angle for `+`/`-`. Default 25.                         |
| `max_depth` | integer         | Iterations to precompute; clamped to `1..=7`. Default 4.    |
| `seed`      | integer         | Reserved for future stochastic rules (deterministic today). |

Turtle vocabulary in the expanded string: `F`/`G` draw forward, `f` moves without
drawing, `+`/`-` turn by `angle_deg`, `[`/`]` push/pop the branch state, any other
character is an inert grammar variable.

### `[generator]` — for `star_pattern`

| Key                 | Values                                                | Notes                        |
|---------------------|-------------------------------------------------------|------------------------------|
| `tiling`            | `square`/`4`/`4.4.4.4`, `hexagon`/`6`/`6.6.6`, `octagon`/`8`/`4.8.8`, `dodecagon`/`12`/`3.12.12` | Star order `n`. Required. |
| `contact_angle_deg` | number                                                | Hankin contact angle. Default 30. |

## Curated line-art presets in this set

- Roses (`parametric_curve`): `rose_bloom`, `rose_web`, `rose_star`, `rose_draw`.
- L-systems (`lsystem`): `lsystem_fern`, `lsystem_arrowhead`.
- Star (`star_pattern`): `star_rosette`.

## Plan 0018 showcase presets

One preset per new engine-wide control, a starting point for authoring your own:

- `rose_zoom` — a rose whose view **zoom** pumps on the bass with a slow sine pan.
- `rose_atmosphere` — a rose over a vignetted **background** gradient.
- `rose_kaleidoscope` — a six-fold **geometry mirror**, reflection toggled on the beat.
- `rose_trails` — a spinning rose smeared into a glowing spiral by the **trails** feedback.
- `fragment_kaleido` — a fragment field folded into an eight-fold screen-space **kaleidoscope**.
- `fragment_smooth` — beat-driven flash/glow **eased** via a `[smoothing]` table.
