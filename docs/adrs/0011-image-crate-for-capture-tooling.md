# ADR-0011 — Use the `image` crate (dev-dependency only) for headless-capture PNG I/O and golden compare

> **Status:** accepted (Plan 0013 closed 2026-07-22)
> **Date:** 2026-07-22
> **Related plan(s):** [0013](../plans/0013-headless-scene-capture.md)

## Context

Plan 0013 adds a headless scene-capture path so the `dev` agent gets visual feedback while
building new geometry/forms, and so scene output can be regression-tested with committed
golden images. That path produces raw RGBA pixels in `core` (a pure wgpu texture readback —
no dependency). Three consumers around it need real image handling:

- **Encode** — the `standalone/examples/shot.rs` CLI turns captured RGBA into a viewable
  `.png` on disk.
- **Decode** — the `core/tests/golden.rs` harness reads committed baseline PNGs back into
  pixels to compare against a fresh render.
- **Compose** — the CLI's `--all` gallery mode tiles many per-preset captures into one
  contact-sheet PNG (resize + blit into a larger buffer).

Two constraints pull against each other. First, **"lightweight is a feature"** (CLAUDE.md):
every crate is a cost, direct deps are pinned to exact versions, and — critically — the
`standalone` crate *is* the shipped `lmv.exe` that Plans 0011/0012 spent effort trimming. A
normal dependency there re-grows the very binary we measured. Second, we still want ergonomic
encode/decode/compose without hand-rolling PNG chunking or montage math.

Two candidate crates: `png` (single-purpose PNG encode/decode, tiny) and `image` (multi-format
encode/decode plus `RgbaImage` buffers, `resize`, and sub-image blitting). `png` covers encode
and decode but leaves the gallery montage (resize + tile into one canvas) as hand-written
buffer arithmetic, and gives core and standalone two different pixel vocabularies.

## Decision

We will adopt **`image`** (exact-pinned) as a **dev-dependency of both `core` and `standalone`**
— never a normal dependency of either. Because it is dev-scoped, it is compiled only for
`cargo test` and `cargo run --example`/`--test`, and is **excluded from the release `lmv.exe`**,
exactly like the `floor.rs` example's isolation in Plan 0012. `core`'s shipped API stays
image-free: the headless render entry point returns raw `Vec<u8>` RGBA, and only the test-scoped
golden harness pulls `image` in to decode baselines. We choose `image` over `png` because the
gallery montage needs `image`'s resize + blit helpers, and a single crate gives core's tests and
the standalone CLI one shared `RgbaImage` type instead of two PNG codecs with divergent buffer
conventions. No cargo *feature* gate is introduced — dev-dependency scope already achieves the
"absent from the release binary" goal more simply than a feature would.

## Consequences

### Positive
- The shipped `lmv.exe` gains **zero** bytes: `image` never enters a non-test, non-example build.
- Golden baselines are committed as ordinary PNGs — viewable in the repo and in PR diffs.
- One pixel vocabulary (`image::RgbaImage`) spans the core golden test and the standalone CLI,
  so encode, decode, and montage share types.
- `--all` gallery montage is a few `image` calls, not hand-rolled row math.

### Negative
- `image` is a heavier crate than `png` (more transitive deps, more codecs we don't use).
  We accept this **only** because dev-dependency scope keeps it out of the artifact whose size
  we care about; the build-time and `Cargo.lock` cost remains.
- It appears in **two** crates' `[dev-dependencies]` (core and standalone), pinned in both —
  two pins to keep in step, both under the exact-version rule.
- Dev-dependency status means the capture CLI is `cargo run --example shot`, not a subcommand of
  the shipped binary. That is intentional (keeps the encoder out of `lmv.exe`) but means the
  feature is a developer/agent tool, not an end-user one.

### Neutral
- Cross-GPU rasterization drift is a golden-image concern, but it is a *Plan 0013* problem
  (software-adapter + tolerance compare), not a dependency-choice one — this ADR is only about
  the crate.

## Alternatives considered

### Alternative A — `png` crate instead of `image`
Smaller and single-purpose, and it covers encode + decode. Rejected because the `--all` gallery
montage (resize each capture, tile into one canvas) would become hand-written buffer arithmetic,
and core's decode side and standalone's encode side would speak two independent PNG APIs rather
than one shared pixel type. The size win is moot once the crate is dev-only.

### Alternative B — `image` as a normal dependency behind a cargo feature
A `capture` feature on `standalone`/`core` gating a normal `image` dep. Rejected because
dev-dependency scope already excludes `image` from the release build with less machinery — a
feature adds a build-matrix axis and the risk of someone shipping with it on, for no gain over
the dev-dep the example/test already compile against.

### Alternative C — no crate; hand-roll a minimal PNG writer
A ~100-line uncompressed-PNG encoder in the standalone, zero deps. Rejected because it gives
encode only (golden compare still needs a decoder), produces bloated uncompressed files, and is
error-prone CRC/zlib code to maintain — a poor trade against a dev-only dependency.

## Notes

Pairs with Plan 0013, which owns the determinism strategy (software/fallback adapter + tolerance
compare + a `LMV_BLESS` baseline-regen path). This ADR is accepted at that plan's close per the
project workflow.
