# Spec — Ring seam + DSP determinism

> **Subsystem:** The lock-free SPSC ring buffer that decouples audio from render, and the pure-function DSP that consumes it (FFT/spectrum, onset, tempo/beat).
> **Source:** `core/src/audio.rs` (ring), `core/src/dsp/` (analysis); ring extraction to `lmv-ring` is Plan 0005.
> **Reconciled-through:** Plan 0003 (DSP enriched with bass/mid/treb + deterministic tempo; ring unchanged since Plan 0001)
> **Governing ADRs:** [0001](../adrs/0001-rust-core-wgpu-cabi-foobar-shim.md) (core owns DSP + the audio/render split); CLAUDE.md non-negotiables; Plan 0005 (Miri UB gate).

## Invariants

- The audio thread MUST NOT block, heap-allocate, lock a contended mutex, log, or do file I/O.
  It hands samples to the ring and returns. An underrun is an audible click; a blocked callback
  is a stutter. (CLAUDE.md non-negotiables)
- The seam between audio and render MUST be the lock-free **SPSC** ring buffer — exactly one
  producer (the audio thread) and one consumer (the render thread). Neither loop is driven
  directly off the other; the ring absorbs the cadence mismatch. (CLAUDE.md)
- The ring MUST be **data-race-free** under concurrent single-producer/single-consumer access.
  This is the invariant Plan 0002 Phase 5 (deferred) / Plan 0005 exists to prove under Miri.
  (Plan 0005)
- DSP analysis (FFT bins, onset envelope, tempo/BPM estimate, bass/mid/treb bands) MUST be a
  **pure function of its input window**: no wall-clock reads, no unseeded randomness. The same
  input window MUST produce the same analysis frame. (CLAUDE.md "determinism where it's
  testable")
- Any visual jitter or randomness, when wanted, MUST be **explicitly seeded** so a scene is
  reproducible from its seed. (CLAUDE.md)
- Sample rate, channel count, and buffer size MUST be validated once where audio enters the
  core; the hot DSP path downstream trusts them. (CLAUDE.md "validate at the boundary")

## Scenarios

- WHEN the audio thread receives a block of PCM frames THEN it writes them into the ring and
  returns without allocating or locking; the render thread reads them on its own cadence.
- WHEN the render thread consumes faster than the audio thread produces (ring empties) THEN the
  consumer gets "no new data" and reuses the last analysis — it does not block the producer.
- WHEN the producer outruns the consumer (ring fills) THEN the overflow policy is applied at the
  ring (oldest samples dropped) rather than blocking the audio thread.
- WHEN a fixed sine-wave window is fed to the FFT path THEN the spectrum places energy in the
  expected bin(s) deterministically — the same window always yields the same bins (the
  behavioral claim the DSP tests defend).
- WHEN the same audio window is analyzed twice THEN the onset envelope, tempo/BPM estimate, and
  band energies are bit-for-bit identical (no wall-clock, no unseeded RNG in the path).
- WHEN `cargo +nightly miri test -p lmv-ring` runs (Plan 0005) THEN the SPSC ring's cross-thread
  test reports no undefined behavior.

## Known gaps / honest nulls

- The ring is verified UB-clean **locally** today (`cargo +nightly miri test -p lmv-core --lib`,
  including the cross-thread SPSC case); the **CI automation** of that check is outstanding and
  is exactly what Plan 0005 wires (by extracting the ring into a wgpu-free `lmv-ring` crate so
  Miri need not compile the wgpu/naga graph).
- This spec does not contract the *tempo estimator's accuracy* (how close BPM is to ground
  truth) — only its **determinism**. Better tempo tracking is a named later roadmap item.
- The overflow/underrun policy is stated behaviorally here; the exact capacity (~100 ms at
  48 kHz per the ring's sizing) and drop mechanics live in `core/src/audio.rs`.
