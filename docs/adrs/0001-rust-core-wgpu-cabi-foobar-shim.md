# ADR-0001 — Rust core, wgpu rendering, C ABI with a C++ foobar shim

**Status:** accepted (2026-07-21)

## Context

We are building a lightweight, real-time music visualizer that must ship as **both**:

1. a **standalone desktop app** on Windows and macOS, and
2. a **foobar2000 plugin** with the same visuals ("parity" is a v1 requirement).

Two forces make the technology choice non-obvious:

- **foobar2000's plugin SDK is C++ and Windows-centric.** Whatever produces the visuals
  must be linkable from C++.
- **"Lightweight" is a first-class goal** — small binaries, low idle CPU/GPU, few
  dependencies — which argues against a heavy runtime (e.g. bundling a browser).

We also want the visual/DSP code written **once** and reused by both frontends, rather
than maintaining two implementations that drift.

## Decision

We will build a **shared native core in Rust** that owns all DSP (FFT/spectrum, beat and
onset detection) and all rendering (a scene graph on top of **wgpu**). The core is
**source-agnostic**: it accepts a stream of PCM frames and knows nothing about where they
came from.

- The **standalone app** is pure Rust (`winit` for windowing, `wgpu` for the surface),
  with OS loopback capture feeding the core.
- The **foobar2000 plugin** is a thin **C++ shim** that links the core through a minimal,
  versioned **C ABI** (the core builds as a `cdylib`/`staticlib` exposing `extern "C"`
  functions: create handle, push samples, render into a context, resize, free). foobar's
  `visualisation_stream` feeds samples across that ABI.
- Rendering goes through **wgpu**, a single cross-platform abstraction that targets
  **Metal** on macOS and **DX12/Vulkan** on Windows. Scene code is written against wgpu and
  never branches on the backend.

There is exactly **one FFI seam** — the C ABI at the plugin boundary. Everything above it
on the standalone side is safe Rust.

## Consequences

**Positive**

- One visual/DSP codebase serves both frontends; parity is structural, not a sync effort.
- Rust gives memory safety in the real-time audio + GPU path, where use-after-free and data
  races are otherwise easy and painful.
- wgpu gives modern GPU access (Metal/DX12/Vulkan) without hand-writing three backends.
- Small, dependency-light binaries — consistent with the "lightweight" goal.

**Negative (the price we pay)**

- **A C ABI seam to design and maintain.** The `extern "C"` surface is a real contract:
  it must stay minimal and versioned, and the C++ side compiles separately, so a mismatch
  is a link/runtime error, not a compile error. Changing the ABI shape is ADR-worthy.
- **The foobar plugin is not pure Rust.** We still write and build C++ for the SDK
  integration; contributors need a C++ toolchain for that target.
- **wgpu is younger than raw OpenGL** and its API moves; we accept occasional churn for the
  cross-platform + Metal payoff.
- **macOS loopback is not solved by this ADR.** Standalone capture on Mac needs
  ScreenCaptureKit or a virtual device (BlackHole); that is deferred to a later plan and is
  independent of this decision.

## Alternatives considered

- **C++ shared core (instead of Rust).** Would link into the foobar plugin with *zero* FFI
  — the most SDK-native path. Rejected because we prefer memory safety in the real-time
  audio/GPU code and a cleaner cross-platform build for the standalone; the single C ABI
  seam is an acceptable price for that.
- **Electron / web stack (WebGL/WebGPU).** Fastest to build with the best tooling. Rejected
  on two counts: it contradicts "lightweight" (heavy binary, high idle cost), and the visual
  code would **not** reuse in the C++ foobar plugin — we'd reimplement it or embed a webview,
  defeating the shared-core goal.
- **OpenGL everywhere (instead of wgpu).** Simplest to write, most examples online. Rejected
  because OpenGL is deprecated on macOS (no Metal performance, uncertain future) and dated;
  wgpu gives us Metal/DX12/Vulkan from one code path.
- **Two separate implementations (no shared core).** Rejected outright — parity would be a
  perpetual manual sync and the whole point is to write the visuals once.
