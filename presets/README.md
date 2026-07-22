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
| `fragment_field`  | `warp` `hue` `zoom` `glow` `flash`                                        |
| `swarm`           | `force` `spin` `burst` `hue` `brightness` `size`                         |
| `parametric_curve`| `n` `d` `samples` `thickness` `hue` `spin` `scale` `brightness` `draw_progress` |
| `lsystem`         | `visible_depth` `rotation` `hue` `draw_progress` `thickness` `scale` `brightness` |
| `star_pattern`    | `variant` `rotation` `hue` `draw_progress` `thickness` `scale` `brightness` |

Unbound parameters fall back to each system's defaults. Unknown parameter names
are ignored.

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
