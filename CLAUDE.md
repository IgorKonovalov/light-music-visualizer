# light-music-visualizer

A lightweight, real-time music visualizer built around one **shared Rust core** that
turns a stream of PCM audio samples into GPU-rendered visuals. Two frontends consume
that core:

- **Standalone app** (Windows + macOS) — pure Rust (`winit` + `wgpu`), fed by OS
  loopback audio capture.
- **foobar2000 plugin** (Windows-first) — a thin **C++ shim** over the core's **C ABI**,
  fed by foobar's own `visualisation_stream` (no loopback needed on that path).

The core is **source-agnostic**: it takes interleaved/mono PCM frames and does not care
whether they came from loopback capture or foobar. That single abstraction is what makes
one visual codebase serve both frontends. Do not leak audio-source specifics into the core.

This file is the orientation map — it says **which part owns what** and **how they hand
off**, not how the code works. Decisions live in `docs/adrs/`; work-in-flight lives in
`docs/plans/`.

## Architecture at a glance

```
                 PCM frames (source-agnostic)
   loopback ----\                              /---- foobar visualisation_stream
                 v                            v
        [ standalone shell ]         [ foobar plugin: C++ shim ]
          (Rust: winit)                  (links core via C ABI)
                 \                            /
                  v                          v
                   +--------- core ---------+
                   |  Rust: DSP + render    |
                   |  - FFT / spectrum      |
                   |  - beat / onset detect |
                   |  - scene graph + wgpu  |
                   +------------------------+
                        wgpu -> Metal (mac) / DX12 · Vulkan (win)
```

Key architectural decisions are recorded as ADRs. The founding one is
[ADR-0001](docs/adrs/0001-rust-core-wgpu-cabi-foobar-shim.md) — Rust core, wgpu rendering,
C ABI, C++ foobar shim. **Read it before questioning the language/GPU/FFI split.**

## Where things live

```
core/                # Rust library crate — the shared brain. DSP + render engine + scenes.
                     #   Exposes both a native Rust API (for the standalone) and a C ABI
                     #   (cdylib/staticlib) for the foobar plugin. NO audio-source code here.
standalone/          # Rust binary crate — winit window + wgpu surface + OS loopback capture.
plugin-foobar/       # C++ shim: foobar2000 SDK integration, links core's C ABI. Windows-first.
docs/
├── adrs/            # NNNN-<slug>.md — architecture decisions + rejected alternatives. Append-only.
│   └── README.md    #   ADR index
└── plans/           # NNNN-<slug>.md — phased implementation plans (what's in flight)
    ├── README.md    #   Plans index: roster + next free number. Read this first each session.
    └── done/         #   Completed plans move here
.claude/
├── settings.json    # Registers the block-broad-git-add PreToolUse hook
└── hooks/           # block-broad-git-add.js — enforces explicit-path staging
```

## How we work (canonical workflow)

This project runs the **lightweight** version of a plan-driven harness. There is no
multi-skill ecosystem yet — one implementer at a time. The loop:

```
interview  ->  ADR (if a real tradeoff)  ->  plan (phased)  ->  implement phase-by-phase  ->  review at plan end
```

- **Interview before writing.** For any non-trivial feature, ask 3-5 tight questions
  (batch them via `AskUserQuestion`) before designing. A one-minute interview beats a
  rewrite. Skip only if the user says "just draft it" — then state what you're guessing.
- **ADR when there's a rejected alternative.** If you can name an option you're *not*
  taking and future-you would want to know why, write an ADR (`docs/adrs/`). If you can't
  name a rejected alternative, you don't need an ADR — just a comment.
- **Plan before implementing.** Non-trivial work gets a numbered plan in `docs/plans/`
  with **ordered phases**, each tagged `**Owner area:**` (`core` / `standalone` /
  `plugin` / `human`). Each phase ships as its own commit with a clear "done when".
- **Review at plan end**, not per phase. Check the implementation against the plan and
  the cross-cutting rules below, then flip the plan to `done` and `git mv` it to
  `plans/done/`. Update `docs/plans/README.md`.

Numbering: sequential, zero-padded 4 digits (`0001`). ADR and plan numbers are independent
sequences. List existing files and take the next number; the plans README tracks the next
free number so you don't have to re-glob.

## Cross-cutting non-negotiables

These apply to every part of the project. They exist because this is **real-time
audio + graphics**, where the usual "just allocate and log it" habits cause glitches.

- **The audio callback is sacred.** The thread that receives capture / `visualisation_stream`
  data must never block, allocate on the heap, lock a contended mutex, log, or do file I/O.
  Hand samples to the core through a lock-free ring buffer (SPSC) and return immediately.
  An underrun is an audible click; a blocked callback is a stutter.
- **Render and audio are decoupled.** Audio arrives at the device's cadence; frames render
  at the display's. Never drive one loop directly off the other — the ring buffer is the seam.
- **Determinism where it's testable.** DSP math (FFT bins, onset envelope, beat estimate)
  is a pure function of its input window. No wall-clock reads, no unseeded randomness inside
  analysis. Visual jitter/randomness, when wanted, is explicitly seeded so a scene is reproducible.
- **The core stays source-agnostic and GPU-abstract.** No WASAPI / ScreenCaptureKit / foobar
  types in `core/`. No raw Metal/DX/Vulkan calls outside the wgpu layer. The whole point of the
  split is swappability; a leak here forfeits it.
- **The C ABI is a contract.** The `extern "C"` surface the plugin links against is versioned
  and minimal: opaque handle, push-samples, render-into-context, resize, free. Changing its
  shape is an ADR-worthy event, not a casual edit — the C++ side is compiled separately.
- **Validate at the boundary, trust inside.** Sample-rate, channel count, and buffer sizes get
  checked once where audio enters the core; the hot path downstream assumes them valid.
- **Lightweight is a feature.** Small binaries, few dependencies, low idle CPU/GPU. Every new
  crate is a cost — justify it. Pin direct dependencies to exact versions in `Cargo.toml`.

## Platform realities (don't rediscover these)

- **Loopback capture is not symmetric.** Windows has first-class WASAPI loopback. macOS does
  **not** — it needs ScreenCaptureKit (macOS 13+) or a virtual device (BlackHole). So
  "capture any app's audio" is Windows-first; the Mac capture path is a later, asterisked phase.
  The foobar-plugin path sidesteps capture entirely (foobar hands us samples), which is one
  reason plugin parity is valuable on Mac.
- **foobar2000's plugin SDK is C++ and Windows-centric.** The plugin is a C++ shim; it does
  not reuse Rust source directly — it links the core's compiled C ABI. Keep that seam thin.
- **wgpu targets differ per OS.** Metal on macOS, DX12/Vulkan on Windows. Write to wgpu; don't
  branch on the backend in scene code.

## Commit hygiene

- **Stage by explicit path — never `git add -A` / `.` / `--all` / `:/`.** A `PreToolUse` hook
  (`.claude/hooks/block-broad-git-add.js`) denies broad staging so stray/untracked files and
  parallel sessions don't get swept in. Run `git status` first; stage only your files.
- **Conventional commits**, one logical change (or one plan phase) per commit.
- **On Windows, commit multi-line messages via the PowerShell tool's single-quoted here-string**
  (`@'...'@`, closing `'@` at column 0) — the Bash tool mangles here-strings. Keep the body plain
  ASCII (straight hyphens, no em-dashes, no internal double-quotes) or git may misparse it.
- **Never rewrite history** (no amend/rebase/reset) and **never push** — the user pushes.

## Pitfalls to avoid

- **Don't put audio-source or platform code in `core/`.** It breaks the one abstraction the
  whole design rests on.
- **Don't allocate or block in the audio callback.** See the non-negotiables — this is the
  #1 source of real-time audio bugs.
- **Don't skip the ADR for cross-cutting decisions.** New dependency, C ABI change, a second
  GPU backend, a new capture mechanism → ADR, even if the edit feels small.
- **Don't implement without a plan for non-trivial work**, and don't review your own work in
  the same session that wrote it — the fresh-context review is where drift gets caught.
- **Trust `git` / `Glob` over stale docs.** If a plan or ADR names a module that isn't there
  (or vice versa), surface the drift rather than papering over it.
