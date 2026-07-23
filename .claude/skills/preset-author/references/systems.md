# Scene & parameter catalogue

> **Snapshot: 2026-07-23. Five scenes exist.** Scenes are engine code and **change as the app
> develops** — new scenes land, params get added, defaults get tuned. Confirm the current set against:
> - valid `system` names → `SystemKind::from_name` in `core/src/preset/schema.rs`
> - a scene's params + defaults → that scene's `set_param` match + `DEFAULT_*` consts in
>   `core/src/render/scenes/**`
>
> `docs/presets.md` is stale (wrong counts) — do not use it. The "typical range" columns below are
> authoring guidance distilled from the shipped presets, not hard engine limits.

**Naming:** a preset's `system = "…"` uses the underscore name (left column). That differs from the
scene's display `name()` (e.g. `system = "lsystem"` renders the scene whose display name is
`"l-system"`). Use the underscore names.

**Param lifecycle:** each frame the renderer calls `reset_params()` (every param → its default) then
applies the preset's bindings. So **any param you don't bind keeps its default** — you only write the
ones you want to drive. All scenes share one iq-style cosine palette; `hue` is a phase offset `0..1`
into that single looping palette, so color language is consistent across every scene (`+ time * k`
gives a slow hue drift).

---

## `fragment_field` — full-screen domain-warp field

A single full-screen shader; all color in the pixel shader (iterated sine-fold warp + cosine palette).
Reads no audio itself — **all** reactivity flows through the expression-bound params. Clears to black.
The right pick for flowing, ambient, nebula/aurora looks.

| Param | Default | Typical range | Controls |
|-------|---------|---------------|----------|
| `warp` | `0.4` | `0.25 – 2.6` | domain-warp fold amount; higher = more distorted, kinetic field |
| `hue` | `0.0` | `0 – 1` (+drift) | palette rotation offset |
| `zoom` | `1.0` | `0.8 – 2.0` | field scale; `>1` zooms in (larger features) |
| `glow` | `0.7` | `0.3 – 1.2` | overall brightness / bloom |
| `flash` | `0.0` | `0 – 1` | additive white flash on top (ride `onset` for transient pops) |

Idiomatic: `warp` on gained bass, `hue` on `time` drift + a little `treb`, `zoom` breathing on `bar`,
`glow` on band energy, `flash` on `onset`. (See `presets/fragment_aurora.toml`.)

## `swarm` — ~10k-particle flow swarm

~10,000 CPU-simulated particles steered by an evolving flow field, drawn as **additive** sprites over
near-black. Energetic, organic, physical. The right pick for kinetic, dancey, particle looks.

| Param | Default | Typical range | Controls |
|-------|---------|---------------|----------|
| `force` | `1.4` | `1.4 – 7` | steering strength toward the flow field (drive from bass) |
| `spin` | `0.3` | `0.3 – 2.3` | how fast the flow field evolves (`field_t = time * spin`) |
| `burst` | `0.0` | `0 – 12` | radial outward kick from center (drive from `beat` for a pulse) |
| `hue` | `0.0` | `0 – 1` (+drift) | palette offset |
| `brightness` | `0.8` | `0.8 – 1.8` | global brightness multiplier |
| `size` | `1.0` | `1.0 – 2.5` | particle size |

Idiomatic: `force`←bass, `spin`←mid, `burst = beat * 11`, `hue` drift + `treb`, `size` bumped on
`beat`. (See `presets/swarm_storm.toml`.) Additive blending means more particles/size = brighter
bloom — watch the perf floor on dense, large settings.

## `parametric_curve` — Maurer-rose line curve

A Maurer rose (`sin(n·theta)` walked at a fixed angular step), resampled every frame into the shared
line renderer. Precise, geometric, hypnotic. Requires `[curve] family = "maurer_rose"`.

| Param | Default | Typical range | Controls |
|-------|---------|---------------|----------|
| `n` | `6.0` | `2 – 12` | rose petal frequency (petal count / symmetry) |
| `d` | `71.0` | `2 – 360` | angular step in degrees — the Maurer "web" density |
| `samples` | `361.0` | `120 – 720` | chord count (clamped to `MAX_SEGMENTS`) |
| `thickness` | `2.0` | `1 – 5` | stroke weight |
| `hue` | `0.6` | `0 – 1` (+drift) | palette offset |
| `spin` | `0.1` | `0 – 1` | rotation rate (`rotation = spin · time`) |
| `scale` | `0.9` | `0.6 – 1.0` | overall size in the frame |
| `brightness` | `1.0` | `0.8 – 1.6` | color multiplier |
| `draw_progress` | `1.0` | `0 – 1` | line-draw-on reveal (prefix of chords); ride `bar` for a per-beat redraw |

`n`/`d` are the shape; small changes redraw the whole figure. Keep `n` and `samples` integer-ish
(use `floor` if driving them). (See `presets/rose_star.toml`.)

## `lsystem` — branching L-system growth

A grammar expanded and turtle-walked into a cached segment buffer **per depth** at load; per frame
just a rotate/scale/color/draw-on transform. Organic, botanical, "growing." Requires a `[generator]`
table (axiom/rules/angle_deg/max_depth/seed — see `grammar.md`).

| Param | Default | Typical range | Controls |
|-------|---------|---------------|----------|
| `visible_depth` | `1.0` | `1 – max_depth` | which cached iteration is drawn — drive with `floor` off a band to **grow** the structure |
| `rotation` | `0.0` | radians | absolute angle (not a rate — multiply by `time` yourself for spin) |
| `hue` | `0.3` | `0 – 1` (+drift) | palette offset |
| `draw_progress` | `1.0` | `0 – 1` | draw-on reveal; ride `bar` for a per-beat redraw |
| `thickness` | `1.8` | `1 – 4` | stroke weight |
| `scale` | `1.0` | `0.7 – 1.0` | overall size |
| `brightness` | `1.0` | `0.8 – 1.6` | color multiplier |

Signature move: `visible_depth = "5 + floor(1 * bass)"` grows a level on a swell. Only depths up to
`max_depth` are built, so `visible_depth` is clamped to what exists. (See `presets/lsystem_fern.toml`.)

## `star_pattern` — Hankin star rosette

A Hankin contact-angle star rosette (`2·n` segments), one cached rosette per contact-angle **variant**
at load. Symmetric, mandala-like, architectural. Requires a `[generator]` table
(tiling/contact_angle_deg — see `grammar.md`).

| Param | Default | Typical range | Controls |
|-------|---------|---------------|----------|
| `variant` | `1.0` | `0 – 2` | selects a precomputed contact-angle variant (pointy↔blunt); snap on `beat` with `floor` |
| `rotation` | `0.0` | radians | absolute angle (multiply by `time` for spin) |
| `hue` | `0.5` | `0 – 1` (+drift) | palette offset |
| `draw_progress` | `1.0` | `0 – 1` | draw-on reveal |
| `thickness` | `2.0` | `2 – 6` | stroke weight |
| `scale` | `1.0` | `0.8 – 1.0` | overall size |
| `brightness` | `1.0` | `0.8 – 1.6` | color multiplier |

Three variants exist (contact-angle offsets `[-24°, 0°, +24°]` around the base). Signature move:
`variant = "floor(2.99 * beat)"` snaps between pointy and blunt on the beat. (See
`presets/star_rosette.toml`.)

---

## What no scene can do today (→ API feedback)

There is no ping-pong/feedback scene (reaction-diffusion, Lenia — designed, Plan 0014/ADR-0012, not
built), no GPU-compute particle scene (attractors/flames — Plan 0016/ADR-0015, not built), no 3D
scene, no boids/walkers as distinct scenes. There is no view zoom/pan, background, trails, mirror, or
kaleidoscope yet (ADR-0018, not landed) and no per-param easing (ADR-0019, not landed). If a look
wants any of these, it is engine work — capture it per `api-feedback.md`.
