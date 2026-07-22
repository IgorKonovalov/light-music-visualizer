# Headless capture & visual QA

The renderer can draw a scene with **no window** — a surface-less wgpu context
draws into an offscreen texture and hands back raw RGBA pixels. Two things are
built on that (Plan 0013):

- a **`shot` CLI** (`standalone/examples/shot.rs`) that writes PNGs an agent can
  read and a metrics report it can parse, and
- a **differential visual-QA harness** in `core/tests/` that hard-tests every
  preset for reactivity, animation, shape sanity, and beat response, with an
  advisory distinctness report and golden-image regression.

A headless render is a **pure function** of `(preset, input, frame-count)` —
scenes are reseeded per capture, `SCENE_DT` is fixed, and the DSP is
deterministic — so renders are reproducible and diff-able.

Everything here is **dev/agent tooling**. The `image` crate is a *dev-dependency*
only (ADR-0011), so the shipped `lmv.exe` is untouched; the CLI is a
`cargo run --example`, not a subcommand of the app.

> Package name note: the standalone crate is `standalone`, so the invocation is
> `cargo run -p standalone --example shot -- …`.

## The `shot` CLI

Render one preset to a PNG (the agent then Reads the file):

```bash
cargo run -p standalone --example shot -- --preset "Aurora" --frames 120 --out shot.png
```

Flags:

| flag | meaning |
|------|---------|
| `--preset <name>` | preset to render (by name, as shown in the report / library) |
| `--set k=v,...` | constant stimulus frame: `bass,mid,treb,onset,bar` (0..1) and `beat` (non-zero = true) |
| `--frames <N>` | frames to advance before capture (default 120) |
| `--size <WxH>` | render size (default 1280x720) |
| `--out <path>` | output PNG (single shot) or dir/file (`--all`) |
| `--all` | contact sheet of every preset, labeled (needs `--out`) |
| `--report [family=<sys>]` | per-family metrics table (`fragment_field` \| `swarm`) |
| `--json` | emit the report as JSON instead of a text table |
| `--signal <kind:param>` | synth-audio filmstrip (see below) |
| `--audio <clip.wav>` | filmstrip from a 16-bit PCM WAV |
| `--strip <N>` | frames tiled along the audio (default 8) |

Bad arguments and unknown presets exit non-zero with a message. It loads the
app's on-disk preset library if present, else the embedded defaults.

### Examples

```bash
# Shot a preset under a loud beat, at a custom size
cargo run -p standalone --example shot -- --preset "Pulse Field" \
  --set bass=1,onset=1,beat=1 --size 960x540 --out pulse.png

# Labeled contact sheet of the whole library
cargo run -p standalone --example shot -- --all --out gallery/

# Metrics report as a text table, or JSON for parsing
cargo run -p standalone --example shot -- --report
cargo run -p standalone --example shot -- --report --json > report.json

# Beat filmstrip from a synthesized click track (no asset needed)
cargo run -p standalone --example shot -- --preset "Pulse Field" \
  --signal click:120 --strip 8 --out click.png

# Filmstrip from a real clip (16-bit PCM WAV)
cargo run -p standalone --example shot -- --preset "Burst" \
  --audio assets/test/clip.wav --strip 8 --out clip.png
```

`--signal` kinds: `click:<bpm>`, `bass:<hz>`, `treble:<hz>`, `noise:<seed>`,
`chord`. The synth path needs no committed asset. `--audio` reads uncompressed
16-bit PCM WAV only (a hand-rolled reader — no decoder dependency); other
encodings are a followup.

> **Test audio is added manually and never committed.** Drop a 16-bit PCM WAV
> into [`assets/test/`](../assets/test/) — that folder is gitignored (only its
> README is tracked), so no licensed audio lands in the repo. Use your own or a
> royalty-free / CC0 clip; factory-library samples are fine to point at on disk
> but must not be committed. The `--signal` path needs no file, so the whole
> audio pipeline can be validated without adding anything.

The `--report --json` schema is a nested object of numbers keyed by
family/preset: per-band `reactivity`, `animation`, `coverage`, the pairwise
`pixel`/`shape` distinctness matrices, and `near_duplicates`.

## The `core/tests/` harness

All differential tests render on the **software adapter** (`prefer_software`) so
they hold on any GPU. Run the whole suite:

```bash
cargo test -p lmv-core
```

Individual tests (add `-- --nocapture` to see the printed diagnostics):

| test | kind | asserts |
|------|------|---------|
| `reactivity` | HARD | every preset moves for at least one band (bass/mid/treb/onset); prints the per-band vector so a dead single binding — e.g. treble — is visible |
| `animation` | HARD | every preset changes between frame N and N+k at fixed audio (not frozen) |
| `sanity` | HARD | every preset lights a minimum coverage and spans ≥2 quadrants against its own background (not blank, not a dot) |
| `beat` | HARD | a 120 BPM click track through the **real** DSP makes a beat-accent preset render differently on-beat vs off-beat; a zeroed beat binding does not |
| `distinctness` | ADVISORY | prints per-family pixel + shape pairwise matrices and flags near-duplicate geometry; never asserts |
| `golden` | HARD (tolerance) | a small matrix matches committed baseline PNGs within a mean + max-outlier tolerance |

`core::signal` (pure, zero-dep) synthesizes the test audio; `core::render::metrics`
(pure) provides `frame_diff`, `struct_diff`, `coverage`, and `quadrant_spread`,
shared by the tests and the CLI report.

### Golden baselines

Golden baselines live in `core/tests/golden/*.png` and are ordinary PNGs
(viewable in the repo / PR diffs). To regenerate them after an intended visual
change:

```bash
LMV_BLESS=1 cargo test -p lmv-core --test golden
```

**Eyeball the regenerated PNGs before committing** — the first baseline is easy
to enshrine wrong. The compare tolerates minor cross-GPU rasterization drift; a
genuine change exceeds it.

## The habit for a new scene

When you add a new scene or preset:

1. **Eyeball it first** — `--preset <name> --out /tmp/new.png` and Read the PNG.
2. **Add the differential cases** — the `reactivity`, `animation`, and `sanity`
   tests iterate the embedded presets automatically, so a new *preset* is covered
   once it's in the default set; a new *system* may need its per-system floor in
   `sanity`. If it's beat-driven, extend `beat`.
3. **Check distinctness** — run `--report` (or the `distinctness` test) to see if
   the new preset is a near-duplicate of an existing one (advisory).
4. **Bless a golden only after eyeballing** — add a case to `golden.rs` and run
   `LMV_BLESS=1` once the PNG looks right.
