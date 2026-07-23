# Craft — what makes a preset beautiful

Reactivity is the easy part; anyone can wire `bass` to a size. **Beauty is composition** — a look that
reads as one coherent thing, moves musically, and stays alive whether the track is loud, quiet, or
between beats. This is the taste the lane exists to bring. None of it is enforced by the engine; it's
judgment, verified by rendering (`render-loop.md`).

## The four time-scales of motion — layer them

A preset that only responds to `bass` pumps like a subwoofer meter — legible but crude. Beautiful
presets **layer motion across the time-scales the audio vocabulary gives you**, so something is always
evolving at every rhythm:

- **Slow evolution (`time`).** A gentle, unending drift so the look never sits still even in silence.
  Almost always on `hue` (`hue = "0.4 + time * 0.03 + ..."`), sometimes on rotation. This is what
  keeps a scene from feeling frozen between beats.
- **Per-beat breathing (`bar`).** `bar` ramps `0→1` between beats — use it for a swell that resets
  each beat: `zoom = "1.0 + bar * 0.25"`, or `draw_progress = "clamp(0.6 + bar * 0.6, 0, 1)"` to
  redraw a figure across each beat. Reads as musical pulse, not a spasm.
- **Beat accents (`beat`).** The `0/1` gate, for a discrete snap on the beat: a size bump
  (`size = "1.1 + beat * 1.2"`), a radial burst (`burst = "beat * 11"`), or a structural swap
  (`variant = "floor(2.99 * beat)"`).
- **Transient flares (`onset`).** Sharper and more frequent than `beat` — for flashes and stabs that
  track the attack of every hit, not just the downbeat (`flash = "clamp(onset * 3, 0, 1)"`).

A look that uses three or four of these at once feels *composed*. A look that uses one feels like a
meter. Aim for at least: a `time` drift on color + one beat-locked motion + one continuous band
response.

## Reactivity that reads musically, not as thrashing

- **Gain-then-bound, always.** Bands read small; `clamp(band * gain, lo, hi)` is the shape of nearly
  every good binding. Pick `gain` so ordinary material reaches the middle of the range and only peaks
  hit the top; pick `hi` so a peak looks *intense*, not *broken*.
- **Keep a floor.** Bind to `base + reactive`, not raw `reactive`. `glow = "0.4 + clamp(...)"` never
  goes fully dark on a quiet passage; `glow = "clamp(...)"` alone flickers to black and looks dead.
  The base is the look at rest; the reactive part is the life on top.
- **Match the driver to the band.** Bass for weight/force/size; treble for color/shimmer/detail; mid
  for the middle (flow speed, spin); `onset`/`beat` for punctuation. Cross-wiring (treble → size)
  usually reads as noise.
- **Don't over-react.** If everything jumps on every beat, nothing stands out. Pick one or two things
  to move hard and let the rest drift. Restraint is what separates a designed look from a light-organ.

## Color

All scenes share one looping cosine palette; `hue` is a `0..1` phase offset into it. So color is about
*where* and *how fast* you sit in that palette:

- **A slow `time` hue drift** (`+ time * 0.02..0.06`) walks the whole palette over a track — gorgeous
  for long sets, and it's what keeps rotation-static scenes alive.
- **A little treble on hue** (`+ clamp(treb * k, 0, 0.3)`) makes color shimmer with the high end
  without losing the base tone.
- **Pick a base hue that suits the mood** — cool blues/greens for ambient, hot oranges/magentas for
  aggressive. Set it as the constant term (`hue = "0.72 + ..."`).
- Beauty is often a *narrow* color journey done well, not a rainbow. A preset that drifts through a
  tight arc of related hues usually reads more elegant than one sweeping the full wheel every second.

## Per-system aesthetics

- **`fragment_field`** — flow and atmosphere. Lives on `warp` (structure) × `zoom` (scale) × `hue`
  (drift). Keep `warp` moderate for nebula-like calm, push it for turbulent energy. `flash` on `onset`
  for lightning in the field. This is your ambient/aurora/nebula instrument.
- **`swarm`** — energy and physicality. `force` gives it drive, `spin` gives the field life, `burst`
  on `beat` gives it a heartbeat. Additive blending means density reads as glow — bright and kinetic.
  Your dance/energy instrument.
- **`parametric_curve`** — precision and hypnosis. The rose is mesmerizing when it slowly `spin`s and
  the `hue` drifts; `draw_progress` on `bar` makes it redraw itself each beat. Keep it geometric and
  clean. Your hypnotic/mathematical instrument.
- **`lsystem`** — growth and nature. The signature is `visible_depth` growing on a bass swell — the
  plant literally grows into the music. Slow `rotation`, botanical. Your organic/generative instrument.
- **`star_pattern`** — symmetry and architecture. Mandala-like; the `variant` snap on `beat` gives a
  crisp structural accent. Slow rotation, breathing brightness. Your sacred-geometry instrument.

## Make it survive a real track, not just the loud frame

The loud-frame still (`--set bass=1,...`) shows the peak; it does not show the preset at rest or
mid-motion. Before you call a preset done, also look at:

- **A quiet frame** (`--set bass=0.1,mid=0.1,treb=0.05`) — does it still look intentional, or does it
  collapse to nothing? The base terms should carry it.
- **A filmstrip** (`--signal click:120`) — does the motion read musically across time, or does it
  strobe? Is the beat response legible?

A preset that's beautiful at peak **and** alive at rest **and** musical in motion is the bar.

## House conventions

- Start the file with a `#` comment: what the scene is and what drives what ("bass swells the warp,
  treble drifts the hue"). Every shipped preset does this; it makes the set readable.
- Name the preset (`name = "…"`) something evocative — it's what shows in the UI and the contact sheet.
- Keep discrete params integer-clean with `floor` when you drive them (`n`, `samples`, `variant`,
  `visible_depth`).
