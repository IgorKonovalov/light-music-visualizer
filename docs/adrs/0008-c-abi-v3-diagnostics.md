# ADR-0008 — C ABI v3: diagnostics query + debug-overlay toggle

> **Status:** accepted
> **Date:** 2026-07-22
> **Related plan(s):** [0011](../plans/0011-diagnostics-and-memory-trim.md) (implemented + closed 2026-07-22)

## Context

Plan 0011 adds a runtime diagnostics harness — rolling render-timing stats, a GPU/memory
readout, an on-screen overlay, and structured logging — so the project can *state* footprint
and frame-time numbers before/after the NFR §12 memory trim instead of guessing. The user's
decision for that plan was **full parity across all three frontends**: the standalone, the core,
and the foobar plugin all surface the same diagnostics.

The core already owns the render seam (`Renderer::render`) and now owns a `diag` module that
computes the stats and can paint the overlay. The standalone reaches all of that through the
native Rust API — no ABI needed. The foobar plugin cannot: it is a C++ shim compiled separately
against `core/include/lmv_core.h`, and today's v2 surface (nine functions, ADR-0003 extended by
ADR-0006) exposes create/free/push/attach/render/resize/cycle/load-presets/version — nothing that
lets the host read a metric or toggle a debug layer. Parity for the plugin therefore requires the
core to expose diagnostics *across the C ABI*.

Two forces shape the addition. First, ADR-0003's governing value is **surface minimality** — every
`extern "C"` function is a permanent compatibility commitment the C++ side links against, and drift
is a link/runtime error, not a compile error. Second, the diagnostics data is a *snapshot of
values*, not a stream: the host wants to (a) turn the overlay on/off and (b) pull the current
numbers to write into its own log. That is naturally two functions plus one plain-old-data struct,
not a callback or a channel.

A real question is whether the overlay toggle needs an ABI function at all: the core could read an
environment variable (`LMV_DEBUG_OVERLAY`) once at `lmv_create` and never grow the surface. It will
do exactly that for the *default* state — but an env var is set-once-at-launch and cannot flip the
overlay live from a foobar context-menu item, which the plan wants. So the toggle is both: env var
for the boot default, one ABI function for live control.

## Decision

We will add **two** functions and **one** struct to the C ABI and bump `LMV_ABI_VERSION` from `2`
to `3`:

> `int32_t lmv_set_debug(LmvHandle* handle, uint32_t flags);`
> `int32_t lmv_get_metrics(LmvHandle* handle, LmvMetrics* out);`
> `struct LmvMetrics { uint32_t struct_size; uint32_t abi_version; float fps; float frame_ms_avg;
>   float frame_ms_p99; uint64_t frames_total; uint64_t frames_dropped; uint64_t gpu_bytes;
>   uint32_t draw_calls; uint32_t reserved; };`

`lmv_set_debug` sets a bitflag set on the handle (`LMV_DEBUG_OVERLAY = 1<<0`, higher bits reserved
and ignored); it is idempotent and cheap, callable from the render-thread role at any time. At
`lmv_create` the core reads `LMV_DEBUG_OVERLAY` from the environment once (a boundary read) to seed
the default flags, so a host that never calls `lmv_set_debug` still honors the env default.
`lmv_get_metrics` fills a **caller-allocated** `LmvMetrics` (no allocation crosses the boundary) with
the current rolling snapshot and returns `LMV_OK`, or a negative `LMV_ERR_*` on a null handle/out.
The caller sets `out->struct_size = sizeof(LmvMetrics)` before the call; the core writes at most that
many bytes and stamps the `struct_size`/`abi_version` it actually wrote, so a future field addition is
an append (larger struct, same function) that an older host still reads safely — the struct is
forward-extensible by size, which is why it leads with `struct_size`.

The struct carries only values the **core** can know: render-timing stats it computes and the GPU
resource bytes it tracks. **Process RSS is deliberately not in it** — that is the host process's
working set (foobar's whole process for the plugin; only meaningful for the standalone's own
process), so each shell reads and logs its own RSS via an OS call, outside the ABI. Neither new
function touches the audio-callback role; both are render-thread-only, like the rest of the surface
except `lmv_push_samples`.

`LMV_ABI_VERSION = 3` becomes the new baseline any consumer diffs against; the v1/v2 shapes stay the
historical record in ADR-0003/0006. The v1→v3 handshake still runs through `lmv_abi_version`, so a
plugin built against an older header loaded on a v3 core (or the reverse) detects the mismatch
instead of calling a function that isn't there.

## Consequences

### Positive
- The foobar plugin reaches diagnostics parity with the standalone: the same overlay (painted by the
  core, so it appears in the plugin automatically once the flag is set) and the same metrics written
  to its own log — through two functions, honoring ADR-0003's minimal-surface ethos.
- `LmvMetrics` leads with `struct_size` + `abi_version`, so later diagnostics fields append without a
  v4 bump — the size-guarded read is forward-compatible by construction.
- `lmv_get_metrics` fills a caller-owned struct: zero allocation across the boundary, safe to poll
  every frame or once a second.
- Gives the ABI a second reason to grow in-crate FFI tests (create → set_debug → get_metrics → assert
  the struct is populated and versions match), continuing to chip at the long-standing zero-CI-coverage
  gap on the FFI (Plan 0001/0002/0006 reviews).

### Negative (the price we pay)
- **The v3 surface is eleven functions plus a public struct to keep stable forever.** The struct in
  particular blesses a *snapshot-of-scalars* model of diagnostics; a future need for time-series or
  per-scene breakdowns is another ABI decision, not a silent reshape of `LmvMetrics`.
- **`lmv_get_metrics` returns core-only values**, so the plugin's log will not carry a "footprint"
  number the way the standalone's will (RSS is host-process-owned). This is an honest limitation of
  parity: the plugin sees frame-time and core GPU bytes, not a process working set that would mean
  "all of foobar," not "us."
- The header stays hand-synced with `ffi.rs` (no cbindgen yet, per ADR-0003), so the two new
  signatures, the struct layout, and the flag/error constants are a review/link-time contract — a
  layout mismatch between the C++ struct and the Rust `#[repr(C)]` struct is a silent memory bug, not
  a compile error. Phase 4's FFI test and a `static_assert(sizeof …)` on the C side are the guard.

### Neutral
- `gpu_bytes` is the sum of GPU resources the core itself allocates (an approximation of GPU
  footprint, not a driver-reported number — wgpu does not expose device memory), documented as such.
- The overlay toggle is dual-sourced (env var for the boot default, `lmv_set_debug` for live control);
  the env read is a once-at-create boundary read, consistent with "validate/read at the boundary."

## Alternatives considered

### Alternative A — Env var only, no ABI functions
Toggle the overlay via `LMV_DEBUG_OVERLAY` and skip metrics-over-ABI entirely; let the plugin measure
its own frame time in C++. Rejected: an env var is set-once-at-launch, so the plugin could never flip
the overlay live from a menu item, and the plugin would then compute frame-time stats separately from
the core — two implementations of the same numbers, defeating the "single source of metrics" point of
putting `diag` in the core. Parity means the plugin reads the *core's* numbers.

### Alternative B — One combined `lmv_debug(cmd, arg, out)` multiplexer
Collapse both operations into a single command-dispatch function to keep the count at one. Rejected:
a `switch(cmd)` multiplexer is a wider *effective* surface hidden behind a narrow signature — it
trades a clear, type-checked two-function contract for an untyped command enum that grows silently and
is exactly the kind of casual-reshape ADR-0003 exists to prevent. Two named functions with typed
signatures are the smaller real commitment.

### Alternative C — Push metrics via a host-registered callback
Have the host register a function pointer the core calls each frame with the metrics. Rejected: a
callback across the C ABI adds a threading/lifetime contract (when is it called, on which thread, can
it be unregistered mid-render) far heavier than a pull-when-you-want snapshot, for a host that only
needs to sample at ~1 Hz for its log and read once per frame for an overlay the core already draws
itself.

## Notes

Supersedes nothing; extends ADR-0003 (v1) and ADR-0006 (v2), which stay the historical record of the
earlier shapes. The wgpu per-OS backend trim in the same plan (Plan 0011) is **not** ADR-worthy —
NFR §12 already names it as the primary memory lever and NFR §2 already fixes DX12 as the Windows
baseline, so dropping the Vulkan/GL fallback introduces no new rejected alternative; it is recorded
in the plan's risks, not here.
