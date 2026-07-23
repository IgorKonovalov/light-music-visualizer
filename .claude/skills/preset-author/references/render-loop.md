# Render-and-verify with the `shot` CLI

> **Snapshot: 2026-07-23.** `shot` is engine tooling and **its flags change** — confirm against the
> arg parser in `standalone/examples/shot.rs` before relying on a flag. A preset you have not rendered
> **with audio injected** is a guess; this loop is what makes the lane trustworthy.

## What `shot` is

A headless capture **example** in the `standalone` crate (landed by Plan 0013). It loads the preset
library, renders a scene to an image without a window, and writes a PNG. It is an example, not a
shipped binary, so it never bloats `lmv.exe`.

```sh
cargo run -p standalone --example shot -- <flags>
```

## The critical gotcha: a bare still is a DEAD still

The default stimulus is **silence** (all bands 0, no beat), and every captured frame sees that same
silence — so a plain `--preset X --out y.png` renders the scene frozen at its defaults, reacting to
nothing. It tells you almost nothing about the preset. **You must supply audio one of two ways:**

1. **`--set` a loud constant frame** → freezes the scene at a chosen excitation level. This is the
   right tool for a **single still** you can judge:
   ```sh
   cargo run -p standalone --example shot -- --preset "Aurora" \
     --set bass=1,mid=1,treb=1,onset=1,beat=1,bar=0.5 --out loud.png
   ```
   `--set` keys: `bass mid treb onset bar` (f32) and `beat` (truthy → `1.0`). `bass=1` is already at
   the top of the useful range (bands read small), so `=1` is your "loud" reference. Vary the set to
   probe different moments (e.g. `--set bass=1,beat=0` for the off-beat, `--set onset=1` for a hit).

2. **`--signal` / `--audio`** → synthesizes real PCM, runs the **real DSP analyzer**, and renders a
   **filmstrip** (frames tiled across time) so you see actual motion and beat response:
   ```sh
   cargo run -p standalone --example shot -- --preset "Storm" --signal click:120 --out strip.png
   ```
   `--signal` kinds: `click:<bpm>`, `bass:<hz>`, `treble:<hz>`/`treb:<hz>`, `noise:<seed>`, `chord`.
   `--audio <clip.wav>` drives from a 16-bit PCM WAV. `--strip <N>` sets how many frames tile (default
   8). Use this to judge *behavior over time* — beat snaps, draw-on reveals, drift.

Rule of thumb: **`--set` for a single still to judge composition/color; `--signal` for motion and
beat response.**

## Getting `shot` to see your DRAFT

`shot` loads its library from the per-user app dir, falling back to the embedded set. **Plan 0015's
`LMV_PRESET_DIR` override is NOT landed** (verify: grep `LMV_PRESET_DIR` in `*.rs` — docs only), so
there is no flag to point `shot` at an arbitrary file today. To render a draft, place it where `shot`
looks:

- **Windows:** `%APPDATA%\light-music-visualizer\presets\<file>.toml`
  (PowerShell: `$env:APPDATA\light-music-visualizer\presets\`)
- **macOS:** `~/Library/Application Support/light-music-visualizer/presets/`
- **Linux:** `$XDG_DATA_HOME/light-music-visualizer/presets/` (or `~/.local/share/...`)

Write the draft there, then `--preset "<name>"` (matches the preset's **`name`** field, not the
filename). Editing the file in place and re-running re-renders it. Note: once that dir has any
compiling preset, `shot` uses the **on-disk** dir (not embedded), so your draft renders alongside
whatever else is in the dir — useful for `--all` comparisons, but keep the dir tidy so the contact
sheet stays readable.

> If Plan 0015 lands while you're using this skill, prefer its `LMV_PRESET_DIR` / single-file flag —
> that's exactly the kind of tooling improvement this loop wants. Re-check `shot.rs`.

## Refreshing an already-seeded preset (the write-if-absent trap)

The section above is for a **new** draft. Editing a preset that already ships (or one you
rendered once, so it's already in the per-user dir) has an extra trap: **first-run seeding
is write-if-absent** — `core/src/preset/mod.rs::seed_dir` *never* overwrites a file that
already exists (verify: read its doc comment, ~line 109). Consequences:

- The **repo `presets/<name>.toml` is the canonical source of truth** — it's also what's
  compiled into the `EMBEDDED` set. Always edit there.
- The per-user copy under `%APPDATA%\light-music-visualizer\presets\` is a **runtime cache**
  the app seeded once and will *never* re-sync. Editing only the repo file is invisible to
  `shot` and to the running app — forever.

So to make an edit actually render, **copy the canonical file over the cached copy** (this is
the step whose absence produces "I changed it but it looks the same"):

```sh
# after editing presets/<name>.toml
cp presets/<name>.toml "$APPDATA/light-music-visualizer/presets/<name>.toml"

# then verify — shot reads the per-user dir:
cargo run -p standalone --example shot -- --preset "<name>" \
  --set bass=1,mid=1,treb=1,onset=1,beat=1,bar=0.5 --out loud.png
```

The "clean" alternative — delete the cached copy and relaunch the app to re-seed — only picks
up your edit **after `cargo build`**, because re-seeding writes the *compile-time* `EMBEDDED`
copy, not your on-disk repo edit. The copy-over avoids the rebuild; prefer it while iterating.

**Symptom this prevents:** "I changed the preset but it looks the same as before" is almost
always an edit that reached the repo file (or vice-versa, only the cache) but not both. Repo =
canonical, per-user dir = cache; keep them in sync with the copy-over.

> When **Plan 0015's `LMV_PRESET_DIR`** lands, this whole dance collapses to pointing shot/app
> at the repo `presets/` dir directly — that override has real, repeated demand, and this trap
> is exactly why. Re-check `shot.rs`; prefer the override the moment it exists.

## Other modes

| Flag | Effect |
|------|--------|
| `--all --out sheet.png` | **contact sheet** — every preset in the library as a labeled thumbnail grid in one PNG. The fastest way to compare a draft against the shipped set, and to offer the user side-by-side directions. |
| `--report` / `--report --json` | a **metrics table** (reactivity / animation / coverage / near-duplicate), no image. Use to sanity-check that a preset actually reacts and isn't a near-dup of an existing one. `family=<system>` filters it. |
| `--frames <N>` | frames advanced before capture (default 120). More frames = later in any `time`-driven animation. |
| `--size <WxH>` | render size (default 1280x720). Render at/near 1080p when judging the real look. |
| `--out <path>` | output PNG (parent dirs auto-created). For `--all`, a `.png` path is used verbatim; any other path is treated as a dir and the sheet lands at `<out>/contact-sheet.png`. |

## The loop in practice

1. Write draft → per-user presets dir.
2. `--set` loud still → look at it. Composition right? Color cohesive? Reacting at all?
3. `--signal click:120` filmstrip → does it move musically over time / on the beat?
4. Tune the `.toml`, re-render. Repeat.
5. To offer the user directions: render 2–3 variants (or `--all`) and show the stills side by side —
   let them pick. This project decides by looking, not by prose.
6. Save the chosen still(s) into the scratchpad or a workspace dir so the user can flip through them.
