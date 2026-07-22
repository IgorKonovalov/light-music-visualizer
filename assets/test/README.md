# Local test audio (not committed)

Drop a short audio clip here to exercise the headless-capture `--audio` path
(Plan 0013). **Audio files in this folder are gitignored** — only this README is
tracked — so nothing licensed ever lands in the repo.

## What to add

A **16-bit PCM WAV** (the `shot` CLI's hand-rolled reader supports uncompressed
16-bit PCM only). A few seconds is plenty; a beat-driven clip shows the most.

Where it comes from is your call, but it must be something you're allowed to keep
locally:

- **Your own** music/loop (e.g. bounce a few bars out of a DAW to 16-bit WAV), or
- A **royalty-free / CC0** clip.

Factory/library samples (Ableton, etc.) are fine to point the CLI at *on disk*
for a quick local test, but are licensed and must **not** be committed or
redistributed.

## Use it

```bash
cargo run -p standalone --example shot -- \
  --preset "Pulse Field" --audio assets/test/<name>.wav --strip 8 --out strip.png
```

Then open `strip.png`. The synthetic `--signal` path (e.g. `--signal click:120`)
needs no file at all, so the audio path can be validated without adding anything
here. See [`docs/capturing.md`](../../docs/capturing.md).
