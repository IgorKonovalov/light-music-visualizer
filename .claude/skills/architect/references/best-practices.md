# Best practices — light-music-visualizer

The longer list of correctness rules for this codebase. The architect checks against these in
Mode 4; `dev` implements against them whether or not a plan restates them. Ordered roughly by
how much damage a violation does.

## Real-time audio safety (the cardinal rules)

- **The audio callback is sacred.** The thread that receives capture data (WASAPI /
  ScreenCaptureKit) or foobar's `visualisation_stream` must never: allocate on the heap, take a
  contended lock, log / `println!`, do file or network I/O, or call anything that might block.
  Its only job is to copy samples into a lock-free ring buffer and return. An underrun is an
  audible click; a blocked callback is a stutter.
- **The ring buffer is the seam.** Audio arrives at the device cadence; frames render at the
  display cadence. They communicate through a single-producer/single-consumer lock-free ring.
  Never drive the render loop directly off the audio callback or vice versa.
- **No panics in the hot path.** `unwrap()` / `expect()` / array-index-out-of-bounds on any
  per-frame audio or render path is a latent crash under real input. Handle or clamp; reserve
  panics for genuine invariant violations at init.

## Layering — the source-agnostic, GPU-abstract core

- **No platform or audio-source types in `core/`.** No `windows`/WASAPI, no ScreenCaptureKit,
  no foobar SDK types, no `winit` window types inside the core. The core takes PCM frames and a
  render target; it does not know where either came from. This is the swappability the whole
  architecture exists for (ADR-0001) — a leak here forfeits it.
- **No raw GPU calls outside the wgpu layer.** Scene code targets the wgpu abstraction and never
  branches on Metal vs DX12 vs Vulkan. Backend selection lives in one place.
- **Shells stay thin.** `standalone/` owns windowing + capture + input and forwards to the core.
  `plugin-foobar/` owns SDK glue and forwards across the C ABI. Neither reimplements DSP or scenes.

## The C ABI is a contract

- The `extern "C"` surface is **minimal and versioned**: opaque handle create/free, push-samples,
  render-into-target, resize — and little else. The C++ plugin compiles against it separately, so
  a mismatch is a link/runtime failure, not a compile error.
- **Changing the ABI shape is ADR-worthy**, not a casual edit. Adding a parameter, changing a
  struct layout, or altering ownership/lifetime semantics all count.
- Ownership and lifetimes across the boundary are explicit and documented: who allocates, who
  frees, whether a pointer outlives the call. No implicit `Box::leak` without a matching free.

## Determinism where it's testable

- **DSP is a pure function of its input window.** FFT bins, onset envelope, and beat/tempo
  estimate depend only on the samples fed in — no wall-clock reads, no unseeded randomness, no
  hidden global state. This is what makes them unit-testable against fixtures.
- **Visual randomness is explicitly seeded.** When a scene wants jitter/noise, the seed is
  explicit so a scene render is reproducible for debugging and testing.

## Validate at boundaries, trust inside

- Sample rate, channel count, and buffer sizes are validated **once**, where audio enters the
  core. The hot path downstream assumes them valid — don't re-check per frame.
- External inputs (a decoded file, a config value, a window handle from the C++ host) get
  checked at the seam; trusted code past the seam does not re-validate.

## Lightweight is a feature

- **Every dependency is a cost.** New crates need justification — prefer the standard library
  and small, focused crates. A visualizer that pulls in a heavy framework has lost the plot.
- **Pin direct dependencies to exact versions** in `Cargo.toml` (`= "x.y.z"`). No caret/tilde
  ranges on direct deps — reproducible builds matter more than automatic minor bumps.
- **Watch idle cost.** Low CPU/GPU when the music is quiet or paused; don't spin a busy loop.

## Security & integrity

- **No secrets in code, logs, commit messages, plans, or ADRs.** There's little secret material
  in a visualizer, but signing certs / notarization credentials (packaging) never land in the repo.
- **Defend against malformed audio.** NaN/Inf samples, zero-length buffers, unexpected channel
  counts, sample-rate changes mid-stream — handle them at the boundary rather than letting them
  produce NaN geometry or a divide-by-zero in the DSP.
- **The plugin runs inside foobar2000's process.** A crash or hang in the plugin takes down the
  user's player. The C ABI boundary must not propagate Rust panics across FFI as undefined
  behavior — catch at the boundary and return an error code.
