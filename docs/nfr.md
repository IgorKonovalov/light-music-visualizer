# Non-functional requirements (v1)

Agreed in the 2026-07-21 architecture interview. These are the numbers behind every
"lightweight", "real-time", and "stable frame rate" in the plans. A done-when that
contradicts this file is a plan bug — surface it, don't guess.

## 1. Performance — adaptive quality

- **Model:** scenes ship **quality tiers**; a **frame-time governor** picks the tier so the
  render loop holds the display's refresh rate on whatever hardware is present. Rich tier on
  discrete GPUs, reduced tier on the iGPU baseline — never a dropped-frame slideshow.
- **Floor:** ≥ 60 fps at 1080p on the baseline hardware (below) at the reduced tier.
- **Background cost:** when the window is minimized or fully occluded, rendering throttles to
  near-zero GPU; DSP may keep running so visuals resume in sync.
- **v1 sequencing:** the MVP (Plan 0001) renders at a single fixed quality that must itself
  hit the floor; the tier system + governor is its own follow-up plan.

## 2. Platform baseline

- **Windows:** Windows 10 1903+, any DX12-capable GPU **including integrated** (~2015+ Intel/AMD iGPU).
- **macOS:** macOS 13+ (ScreenCaptureKit floor), Metal via wgpu.
- **foobar2000:** current stable release, Windows only (per ADR-0001).
- Scene code never branches on backend or OS; the baseline constrains shader features globally.

## 3. Latency — audio to visual

- **Budget: < 60 ms end-to-end** from audible beat to visible reaction (~3 frames @ 60 Hz).
- Working allocation (rough, tune in Plan 0001 Phase 3-4): capture/delivery ≤ 15 ms,
  ring-buffer read-behind ≤ 20 ms, FFT hop ≤ ~11 ms (512 samples @ 48 kHz, window ≤ 2048),
  render + present ≤ 1-2 frames.
- The ring buffer may hold more than 60 ms of *capacity*; the requirement is that the DSP
  reads near the write head, not that the buffer is small.

## 4. Size and dependencies

- **Soft cap ~10 MB** for the standalone release exe; plugin DLL in the same ballpark.
  wgpu is the accepted fixed cost; little else is.
- Release profile: LTO on, symbols stripped, exact-version pins for direct deps.
- Gate: any new crate pulling > ~20 transitive deps needs a stated justification (comment in
  `Cargo.toml` or, if cross-cutting, an ADR).

## 5. Real-time safety (testable restatement)

- The audio callback (WASAPI / ScreenCaptureKit / `visualisation_stream` thread) performs
  **zero heap allocation, zero locks, zero logging, zero file I/O**. Seam is the lock-free
  SPSC ring buffer.
- No panics (`unwrap`/`expect`) on per-frame audio or render paths.

## 6. Determinism

- DSP outputs (spectrum bins, onset envelope, beat estimate) are pure functions of the input
  window — no wall clock, no unseeded randomness. Visual randomness is explicitly seeded.

## 7. CI

- GitHub Actions from the start (right after the workspace scaffold): Windows + macOS
  runners running `cargo build`, `cargo test`, `cargo clippy --all-targets -- -D warnings`,
  `cargo fmt --all --check` on every push.
- GPU rendering and live audio cannot run in CI — those checks stay manual on real hardware.

## 8. Distribution (v1)

- **GitHub release zip**: unsigned standalone exe + a packaged `.fb2k-component` for the
  plugin. No installer, no code signing in v1 (SmartScreen warning accepted). Signing, if
  ever, is a later plan + human task.

## 9. Test hardware matrix (what the user has)

| Machine | Validates |
|---------|-----------|
| Primary Windows dev box | Standalone Windows path, plugin, day-to-day dev |
| Older Windows PC (iGPU) | The performance floor (§1) on baseline hardware (§2) |
| Mac, macOS 13+ | macOS standalone path (Metal + ScreenCaptureKit) — build/test is a human-in-the-loop step |
| foobar2000 (installed) | Plugin loading + `visualisation_stream` behavior |

## 10. Live performance (added in the 2026-07-21 follow-up interview)

The primary real-world use is **live DJ shows**: the app renders to a projector/LED screen
while a DJ mixes. This adds:

- **Session stability:** no crash, leak, or visual degradation over a ≥ 4-hour continuous
  session. A soak test becomes part of the live-features plan's done-when.
- **Inputs (all three, core stays source-agnostic):** loopback (DJ software on the same
  machine), **line-in via an audio interface** (cable from the mixer's booth/rec out — the
  robust stage setup; needs a capture-device path alongside loopback), and the foobar plugin.
- **Scene triggers, layered:** auto-rotate (MilkDrop-style timing, biased toward energy
  shifts/drops) as the baseline; **manual trigger** (hotkey, MIDI worth exploring) as the
  override; **best-effort track-change detection** (long-window spectral/tempo novelty) as an
  experimental extra — never the only mechanism, since beatmatched blends have no hard boundary.
- **Projector output is first-class:** fullscreen-on-chosen-display matters more than desk
  UX; it moves early in the roadmap.
- **Scenes are presets, not code (target state):** visualizations will be authored as
  lightweight MilkDrop-akin preset files with an optional scripting layer for staged,
  coherent per-track arcs and generative systems (walkers, flocks, 3D). Exact shape under
  exploration; the decision will land as an ADR before the preset-engine plan is drafted.
  Plan 0001's built-in Rust scenes remain the walking skeleton and later become the
  rendering vocabulary presets drive.

## 11. v1 UX scope (confirmed requirements, post-MVP plan)

All four are v1 requirements, delivered as their own plan after the Plan 0001 MVP:

- Fullscreen toggle (borderless, hotkey).
- Multi-monitor choice (pick the display to fullscreen on).
- Always-on-top / mini mode.
- Settings persistence (last scene, window size/position/mode, quality tier — small config file).
