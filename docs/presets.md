# Preset authoring guide & library reference

A **preset** is a small text file that describes one visual: it names a built-in
rendering **system** and binds each of that system's **parameters** to a short
**expression** over the live audio analysis. Editing a preset needs no Rust, no
rebuild, and no shader knowledge — you change a line, save, and the running app
picks it up.

This document is the human-readable reference for that format: what ships today,
the exact vocabulary you can use, and where the files live on disk. It is the
authoring counterpart to [ADR-0002](adrs/0002-layered-preset-architecture.md)
(the layered preset architecture) and [Plan 0007](plans/done/0007-curated-preset-library.md)
(the curated library + seeding). Only **layers 1-2** of ADR-0002 exist today —
TOML data presets over a pure expression language. Layer 3 (Rhai scripting),
cross-preset blending, and the other built-in systems are deferred.

> **Accurate as of 2026-07-22**, against the 10-preset curated set. If you add,
> rename, or retire a preset, see [Keeping this current](#keeping-this-current)
> — this catalog is hand-maintained.

---

## Quickstart: your first preset

1. **Find your preset directory** (see [Where preset files live](#where-preset-files-live)).
   On Windows that is `%APPDATA%\light-music-visualizer\presets`. Both the
   standalone app and the foobar2000 plugin read this same folder — it is seeded
   with the curated set on first run.

2. **Copy an existing preset** as a starting point. `swarm_flow.toml` (a calm
   particle swarm) and `fragment_aurora.toml` (a slow warp field) are the
   friendliest bases:

   ```
   copy swarm_flow.toml   my_first.toml
   ```

3. **Edit the bindings.** Open `my_first.toml` and change an expression — for
   example make the beat kick harder:

   ```toml
   system = "swarm"
   name   = "My First"

   [params]
   force      = "1.2 + clamp(bass * 20, 0, 4)"
   spin       = "0.3 + clamp(mid * 6, 0, 1.2)"
   burst      = "beat * 9"          # was beat * 5 — a bigger blast on each beat
   hue        = "time * 0.02 + clamp(treb * 4, 0, 1)"
   brightness = "0.6 + clamp(bass * 6, 0, 0.8)"
   size       = "1.0 + beat * 0.6"
   ```

4. **Save and watch.** The standalone app polls the folder every ~500 ms and
   hot-reloads on any change (no restart). Press **Space** to cycle to your new
   preset; the window title shows the active preset name and system. If the file
   has a typo, the app reports it and keeps the last good set — it never crashes
   on a bad preset.

That is the whole loop: copy, edit an expression, save, cycle. The rest of this
guide is the reference behind each of those choices.

---

## The shipped library

Ten curated presets ship today — five per built-in system, arranged as a
loudness/energy spread from calm to aggressive within each system. The
`File` column is the name under `presets/` (repo) and in your seeded directory.

### Fragment field — fullscreen domain-warped light field

| File | Name | Character |
|------|------|-----------|
| `fragment_glacier.toml` | Glacier | The quiet end. A wide, slow field with barely any warp, a long zoom breath on the beat phase, and a cold hue that drifts almost imperceptibly. |
| `fragment_aurora.toml` | Aurora | Slow and flowing. Bass swells the warp, treble drifts the hue, the beat phase gently breathes the zoom. |
| `fragment_ember.toml` | Ember | Warm and glow-forward. Mids build a steady bloom that bass swells; the hue sits low (reds/oranges) drifting slowly; warping stays gentle. Bright but unhurried. |
| `fragment_pulse.toml` | Pulse Field | Tighter and beat-forward. Each beat kicks the warp and every onset flashes the field; mids push the hue around. |
| `fragment_warp.toml` | Warp Drive | The loud end. Treble tears the field into fast domain-warp, each beat adds a shove, onset snaps the zoom, and the hue races. |

### Swarm — ~10k-particle CPU flow-field swarm

| File | Name | Character |
|------|------|-----------|
| `swarm_drift.toml` | Drift | The quiet end. Gentle force, slow spin, large soft particles easing around a slowly evolving field. Bass gives a slow breathing motion; beats only nudge, no hard bursts. |
| `swarm_flow.toml` | Flow | A calm swarm. Bass steers harder, mids evolve the flow field, treble tints the palette, and each beat gives a small outward nudge. |
| `swarm_dense.toml` | Dense | A tight, fast swarm: high spin churns the field, particles ride small and bright, treble tints them, beats give a crisp shove. Reads as a shimmering cloud rather than distinct dots. |
| `swarm_burst.toml` | Burst | Beat-driven explosions over a faster-evolving field. Harder steering and a big radial shove on every beat. |
| `swarm_storm.toml` | Storm | The aggressive end. Hard steering, fast spin, and a big radial blast on every beat over a bright, fast-shifting palette. Bass drives the force; treble the color. |

---

## Anatomy of a preset file

A preset is a TOML file with a two-line header and a `[params]` table:

```toml
system = "fragment_field"   # required — which built-in system to drive
name   = "Aurora"           # optional — display name (defaults to the system name)

[params]                    # each key is a system parameter; each value is an expression string
warp  = "0.3 + clamp(bass * 14, 0, 1.8)"
hue   = "time * 0.03 + clamp(treb * 5, 0, 1)"
zoom  = "1.0 + bar * 0.25"
glow  = "0.4 + clamp((bass + mid) * 8, 0, 1.1)"
flash = "clamp(onset * 3, 0, 1)"
```

Rules:

- **`system`** must be one of the known system names (`fragment_field` or
  `swarm` today). An unknown system rejects the whole file.
- **`name`** is free text shown in the standalone title bar. If omitted, the
  system name is used.
- **`[params]`** binds parameters by name to expression strings. Every value is
  a **string** (quote it), even a bare number: `warp = "0.4"`, not `warp = 0.4`.
- **Unbound parameters** fall back to the system's default (listed below), so you
  only need to write the parameters you want to drive. Order does not matter —
  bindings are sorted by name at load for determinism.
- **Unknown parameter names are silently ignored** (they do not error), so a typo
  in a parameter name fails quietly by doing nothing — check spelling against the
  tables below if a parameter seems to have no effect.

Every value is evaluated **once per frame** and applied to the system before it
renders. There is no per-frame state you can accumulate in a preset — an
expression is a pure function of the current analysis frame plus the clock.

---

## Built-in systems and their parameters

Both systems expose a fixed set of named `f32` parameters. The **Default** column
is the value used when a preset does not bind that parameter.

### `fragment_field`

A fullscreen, Shadertoy-style domain-warped field with an iq-style cosine
palette. Purely parameter-driven — the audio reaches it only through your
expressions.

| Parameter | Default | What it does |
|-----------|---------|--------------|
| `warp`  | `0.4` | Domain-warp fold amount. Higher = more distorted, kinetic field. Curated presets range ~`0.25`–`2.6`. |
| `hue`   | `0.0` | Palette rotation (offset into the looping cosine palette). Drift it slowly with `time * k` for a wandering color. |
| `zoom`  | `1.0` | Field scale. `> 1` zooms in (larger features); a slow `bar`-driven ramp reads as "breathing". |
| `glow`  | `0.7` | Overall brightness / bloom multiplier. |
| `flash` | `0.0` | Additive white flash on top of the field, ~`0`–`1`. Drive from `onset` for a transient accent. |

### `swarm`

~10,000 CPU-simulated particles steered by an evolving flow field, drawn as
additive points. Simulation runs in Rust; your parameters shape its behavior.

| Parameter | Default | What it does |
|-----------|---------|--------------|
| `force`      | `1.4` | Steering strength toward the flow field — how hard particles are pulled along it. |
| `spin`       | `0.3` | How fast the flow field itself evolves over time. Higher = a more churning, restless field. |
| `burst`      | `0.0` | Radial outward kick from center. Drive from `beat` (e.g. `beat * 9`) for an explosion on each beat. |
| `hue`        | `0.0` | Palette offset added to every particle's base color. |
| `brightness` | `0.8` | Global brightness multiplier over the per-particle brightness. |
| `size`       | `1.0` | Particle size multiplier. |

> Legacy scenes (`spectrum`, `pulse`, `starfield`) exist in the renderer's cycle
> but are **not** preset-driven — they take no parameters and cannot be targeted
> by a preset file. Only `fragment_field` and `swarm` are addressable from a
> preset today.

---

## The expression language

Each parameter value is a tiny arithmetic expression, compiled once when the
preset loads and evaluated every frame. It is deliberately small: pure,
allocation-free, and total (evaluation never panics), so it is safe to run per
parameter per frame during a live show.

### Grammar

Standard arithmetic with the usual precedence:

- **Operators:** `+`  `-`  `*`  `/`, unary `-` / `+`, and parentheses `( )`.
- **Numbers:** decimal `f32` literals (`0.3`, `14`, `1.8`).
- **No comparisons, conditionals, or constants** (no `if`, `>`, `pi`, etc.).
  Shape reactivity with `clamp`, `min`, `max`, and `lerp` instead.

There is no `time`-independent randomness and no way to read wall-clock time —
the only clock is `time`, the renderer's shared scene clock, so a preset is
reproducible given the same audio.

### Variables

Seven read-only variables carry the live audio analysis into your expressions:

| Variable | Meaning | Notes |
|----------|---------|-------|
| `bass` | Mean magnitude in the bass band (~20–250 Hz). | **Raw and small** — multiply up (e.g. `bass * 14`) and clamp. |
| `mid`  | Mean magnitude in the mid band (~250–4000 Hz). | Same scale caveat as `bass`. |
| `treb` | Mean magnitude in the treble band (~4–18 kHz). | Same scale caveat; treble reads smallest of the three. |
| `onset` | Spectral-flux onset envelope for this hop. | A transient/attack strength, not a level — spikes on hits. |
| `beat` | `1.0` on a hop where a beat fired, else `0.0`. | A gate: `beat * k` adds `k` only on beat frames. |
| `bar` | Beat phase in `[0, 1)`: `0` on each beat, ramping to the next. | A sawtooth that "breathes" between beats. |
| `time` | The scene clock in seconds (monotonic). | Use `time * k` for slow drift; `k` sets the speed. |

The band values (`bass`/`mid`/`treb`) are raw mean magnitudes normalized so a
full-scale sine reads near `1.0`, but real program material reads far lower — so
curated presets consistently apply their own gain and then clamp to a bounded
range. That is the central idiom (below).

### Functions

| Function | Args | Result |
|----------|------|--------|
| `sin(x)` | 1 | Sine of `x` (radians). |
| `abs(x)` | 1 | Absolute value. |
| `floor(x)` | 1 | Largest integer ≤ `x`. |
| `min(a, b)` | 2 | Smaller of `a`, `b`. |
| `max(a, b)` | 2 | Larger of `a`, `b`. |
| `clamp(x, lo, hi)` | 3 | `x` bounded to `[lo, hi]`. Total even if `lo > hi` (implemented as `max` then `min`). |
| `lerp(a, b, t)` | 3 | Linear blend `a + (b - a) * t`. |

Calling a function with the wrong number of arguments, or referencing an unknown
name, is a **compile error** — the preset is rejected at load and the app keeps
the previous good set (it does not crash). Division by zero yields `inf`/`NaN`
rather than panicking, but you should avoid it — a `NaN` parameter produces
undefined-looking visuals.

### Idioms (patterns from the curated set)

- **Gain-then-bound** — turn a small raw band into a usable range:
  ```
  clamp(bass * 14, 0, 1.8)
  ```
  Multiply the raw band up, then clamp so a loud passage can't blow the parameter
  out. Nearly every reactive binding is a variant of this.

- **Baseline + reactive** — a resting value plus an audio-driven add:
  ```
  0.4 + clamp((bass + mid) * 8, 0, 1.1)
  ```
  The constant is what you see in silence; the clamped term is the reaction.

- **Slow drift** — a parameter that wanders on its own:
  ```
  time * 0.03
  ```
  Small coefficients (`0.008`–`0.08` in the library) set how fast a hue rotates.

- **Beat gate** — add something only on beat frames:
  ```
  0.5 + beat * 0.8
  ```
  `beat` is `0` most frames and `1` on a beat, so this jumps by `0.8` on each beat.

- **Beat-phase breathing** — a smooth ramp between beats:
  ```
  1.0 + bar * 0.25
  ```
  `bar` sweeps `0 → 1` between beats, so `zoom` eases up and resets each beat.

---

## Where preset files live

There are three copies of the curated set, and understanding the flow explains
why "edit once, both frontends see it" works.

```
  presets/*.toml                 core/src/preset/mod.rs                per-user preset dir
  (repo, source of truth)  ──>   EMBEDDED = include_str!(...)   ──>    seeded on first run,
                                 (compiled into the binary)            then loaded + watched
```

1. **`presets/` at the repo root — the source of truth.** These ten `.toml`
   files are what a contributor edits. Nothing reads them at runtime directly.

2. **`core/src/preset/mod.rs` — `EMBEDDED`.** The core `include_str!`s each
   `presets/*.toml` at build time into the `EMBEDDED` array, so the compiled
   binary always carries the curated set. `default_presets()` parses these as the
   fallback the C-ABI / foobar path renders even with no preset directory
   present.

3. **The per-user directory — what actually gets loaded.** On first run each
   frontend **seeds** this directory (writes every embedded preset that isn't
   already there — **never overwriting** your edits) and then loads and watches
   it. The standalone and the foobar plugin resolve the **same** path, so a
   preset you edit shows up in both.

   | OS | Preset directory |
   |----|------------------|
   | Windows | `%APPDATA%\light-music-visualizer\presets` |
   | macOS | `~/Library/Application Support/light-music-visualizer/presets` |
   | Linux/other | `$XDG_DATA_HOME/light-music-visualizer/presets` (or `~/.local/share/light-music-visualizer/presets`) |

### Loading, cycling, and hot-reload

- **Seeding is write-if-absent.** Your edits to a seeded preset survive
  re-seeding. The flip side: a curated preset changed in a **new release** does
  **not** replace the copy already on your disk — delete that file and relaunch
  to get the updated version (there is no "refresh curated" button yet).
- **Hot-reload (standalone).** The app polls the directory ~every 500 ms and
  reloads on any change. A malformed file is reported (to stderr / the load
  report) and the last good set is kept — a bad edit never crashes a running
  visual.
- **Cycling.** Standalone: **Space** cycles to the next preset (title bar shows
  the name). foobar2000: **Space**, or right-click the visualization → **Next
  scene**.
- **foobar loads on init.** The plugin calls the core's `lmv_load_presets` (C ABI
  v2, [ADR-0006](adrs/0006-c-abi-v2-preset-loading.md)) against the shared
  directory when it starts, so it seeds and renders the same library — no
  loopback capture needed on that path.

---

## Keeping this current

This catalog is **hand-maintained** — nothing regenerates it. Adding, renaming,
or retiring a preset touches **four** places; update all of them in the same
change so the repo, the binary, the tests, and this doc never drift:

1. **`presets/<name>.toml`** — the preset file itself.
2. **`core/src/preset/mod.rs`** — add (or remove) the matching `EMBEDDED` entry
   **and** update the array length in its type (`[(&str, &str); 10]`).
3. **`core/tests/preset.rs`** — the count in `embedded_default_presets_all_parse`
   (currently asserts `10`) guards that every shipped preset compiles; keep it in
   step with the library size.
4. **`docs/presets.md`** (this file) — add/adjust the row in
   [The shipped library](#the-shipped-library), and bump the "accurate as of"
   date near the top.

If you add a **new built-in system** or a **new parameter** to an existing one,
also extend [Built-in systems and their parameters](#built-in-systems-and-their-parameters);
if you add an **expression variable or function**, extend
[The expression language](#the-expression-language). A new system or a change to
the expression grammar is ADR-territory (ADR-0002 fixed the current model) — flag
it rather than quietly widening the vocabulary here.

---

## Related documents

- [ADR-0002 — Layered preset architecture](adrs/0002-layered-preset-architecture.md):
  the data/expression/script model and why it is layered.
- [ADR-0006 — C ABI v2 preset loading](adrs/0006-c-abi-v2-preset-loading.md):
  how the foobar plugin reaches the shared library.
- [Plan 0007 — Curated preset library](plans/done/0007-curated-preset-library.md):
  the seeding, per-user directory, and curated set of ten.
- [Plan 0008 — Preset browse overlay](plans/0008-preset-browse-overlay.md):
  in-app browse/select UX (follow-up to the loading foundation above).
</content>
</invoke>
