# Project context — dev's view

Where things live and the canonical commands. Trust `Glob`/`git` over this when they disagree.

## Repo layout (Cargo workspace)

Plan 0001 builds this out; not all exists yet. Trust `Glob` for what's real.

```
core/            # Rust library crate — DSP + render + scenes + C ABI.
                 #   crate-type = ["rlib", "cdylib", "staticlib"]
  src/audio.rs   #   ring buffer + source-agnostic sample intake
  src/dsp/       #   fft.rs, onset.rs — pure, deterministic, unit-tested
  src/render/    #   wgpu device/surface/context
  src/scenes/    #   Scene trait + spectrum/beat scenes
  src/ffi.rs     #   extern "C" surface for the plugin
  include/       #   C header for the C++ side
standalone/      # Rust binary — winit + wgpu + loopback (capture_win.rs / capture_mac.rs)
plugin-foobar/   # C++ shim — foobar2000 SDK glue, links core's C ABI (Windows-first)
docs/adrs/  docs/plans/  docs/diagrams/
```

## Canonical commands (run from repo root)

| Task                         | Command |
|------------------------------|---------|
| Build all                    | `cargo build` |
| Run the standalone           | `cargo run -p standalone` |
| Test all / just core         | `cargo test` / `cargo test -p core` |
| Lints (errors)               | `cargo clippy --all-targets -- -D warnings` |
| Format check / apply         | `cargo fmt --all --check` / `cargo fmt --all` |
| Build C-ABI artifacts        | `cargo build -p core` (cdylib/staticlib; header via cbindgen if configured) |

All four of build / test / clippy / fmt-check must be green before you commit a phase, unless the
phase's done-when says otherwise.

**foobar plugin (Windows, C++):** built under `plugin-foobar/` with its own project/toolchain,
linking the core's staticlib + generated header. The exact invocation is pinned when Plan 0001's
plugin phase lands — read that phase and the plugin's README rather than guessing.

## Ownership map

- **`dev`** (you) — all code: `core/`, `standalone/`, `plugin-foobar/`.
- **`architect`** — all of `docs/`: plans, ADRs, diagrams, reviews.

Phase owner vocabulary: **`dev`** (all code) and **`human`** (a task only the user can do — a
product call, a signing cert, installing a system audio driver like BlackHole). There are no
sibling implementer skills, so you never hand off to another implementer mid-plan — only back to
architect at the end.

## Rules you implement against (from the architect's best-practices.md)

- **Audio callback is sacred** — no alloc/lock/log/IO on the capture or `visualisation_stream`
  thread; copy into the ring buffer and return.
- **Source-agnostic core** — no WASAPI/ScreenCaptureKit/foobar/winit types in `core/`.
- **wgpu-only rendering** — no raw Metal/DX/Vulkan outside the wgpu layer; scenes don't branch on
  backend.
- **Deterministic DSP** — FFT/onset/beat are pure functions of the input window; seed any visual
  randomness.
- **C ABI is a contract** — minimal, versioned, explicit ownership/lifetimes; don't let Rust
  panics cross FFI as UB (catch at the boundary, return an error code).
- **Quantified NFRs live in `docs/nfr.md`** — 60 fps @ 1080p floor, < 60 ms audio→visual
  latency, ~10 MB soft size cap, callback safety rules. Plans cite it by section; when a
  done-when names a number, that file is where it comes from.
