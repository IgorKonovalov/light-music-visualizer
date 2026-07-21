# ADR-0003 — C ABI v1 surface

**Status:** accepted (2026-07-21)
**Related plan(s):** [0001](../plans/done/0001-core-and-standalone-mvp.md) (Phase 6)

## Context

ADR-0001 fixed that the foobar2000 plugin links `lmv-core` across a single, minimal,
versioned **C ABI**, and sketched the surface as "create handle, push samples, render into a
context, resize, free." Plan 0001 Phase 6 carried the same sketch (~5 functions). When the
ABI was actually implemented and then consumed by the Phase 8 plugin, the surface came out at
**eight** functions. CLAUDE.md and ADR-0001 both call this `extern "C"` surface a *contract*
whose shape is "ADR-worthy," so the v1 shape and the reasons it is wider than the sketch
belong in the durable record rather than only in code comments.

The C++ shim compiles against `core/include/lmv_core.h` separately from the Rust crate, so a
signature or error-code drift is a link/runtime error, not a compile error. That makes the
exact frozen surface, and the intent behind each function, worth pinning now — before any
second consumer (or a v2) exists.

## Decision

The v1 C ABI (`LMV_ABI_VERSION = 1`) is exactly these eight functions, defined in
`core/src/ffi.rs` and mirrored in `core/include/lmv_core.h`:

> `lmv_abi_version`, `lmv_create`, `lmv_free`, `lmv_push_samples`, `lmv_attach_window`,
> `lmv_render`, `lmv_resize`, `lmv_cycle_scene`.

Three points where this differs from the pre-implementation sketch, and why:

- **`render_into` is split into `lmv_attach_window` (create the surface once, from a host
  window handle) + `lmv_render` (draw one frame).** A host attaches its window once and draws
  many frames; folding both into one call would recreate the surface per frame or hide a
  first-call special case. The split is the honest shape of the render seam.
- **`lmv_abi_version` is added** as a runtime version handshake against the compile-time
  `LMV_ABI_VERSION`, so a plugin built against one core can detect a mismatched core instead
  of failing obscurely. Standard versioned-ABI hygiene; near-zero surface cost.
- **`lmv_cycle_scene` is added** so the plugin reaches scene parity with the standalone
  (Space cycles scenes) through the ABI rather than requiring an ABI bump — and thus a new
  ADR — the first time the plugin needs scene control.

Any change to this set (add, remove, or reshape a function; change an error code's meaning)
bumps `LMV_ABI_VERSION` and is recorded in a superseding ADR. Adding a scene to the built-in
roster is **not** an ABI change — `lmv_cycle_scene` already covers "advance through whatever
scenes the core ships."

## Consequences

### Positive
- The v1 surface and its rationale are pinned; a second consumer or a v2 has a baseline to
  diff against, and `lmv_cycle_scene`'s presence is explained rather than mysterious.
- Scene parity across both frontends needs no future ABI bump for more built-in scenes.
- The runtime version query lets a host fail fast and legibly on a core mismatch.

### Negative (the price we pay)
- **A slightly larger contract to keep stable.** Eight functions is still minimal, but every
  one is now a compatibility commitment — `lmv_cycle_scene` in particular ties us to a
  cycle-through-a-flat-roster model of scene selection. If scene *selection* later wants to be
  richer (pick-by-name, preset addressing under ADR-0002's engine), that is a v2 ABI decision,
  not a silent edit.
- The header stays hand-synced with `ffi.rs` (no cbindgen in v1), so drift is caught only by
  review or at link time — a known, accepted maintenance cost recorded here.

### Neutral
- `lmv_attach_window` is Windows-only in v1 (returns `LMV_ERR_UNSUPPORTED` elsewhere), matching
  ADR-0001's "plugin is Windows-first." The signature is platform-neutral (`void* hwnd`), so a
  future host platform is an implementation change, not necessarily an ABI change.

## Alternatives considered

### Alternative A — Hold the ABI to the 5-function sketch; add scene control later
Ship only create/free/push/render/resize, and introduce scene switching when the plugin first
needs it. Rejected: the plugin needs scene parity *in v1* (it is a stated goal), so "later" is
"now," and deferring `lmv_cycle_scene` would force a `LMV_ABI_VERSION` bump plus a superseding
ADR during Plan 0001 itself — ceremony with no benefit over recording it here once.

### Alternative B — A single generic `lmv_control(handle, code, arg)` escape hatch
Collapse `cycle_scene` (and future verbs) into one opaque command entry point to keep the named
surface tiny. Rejected: it trades a small, type-checked, self-documenting surface for a
stringly/enum-typed side channel that the header cannot describe and the compiler cannot check —
exactly the drift risk the C ABI discipline exists to avoid. If the verb count ever grows enough
to justify it, that is its own ADR.

## Notes

Validated by the Phase 8 plugin smoke test (loads in foobar2000 2.x x64, renders all three
scenes, Space cycles). No dedicated C smoke program or automated FFI test exists yet — the ABI
currently has no CI coverage (the C++ side is not built in CI); adding a minimal in-crate test
that drives create/push/free is tracked as a follow-up (Mode 4 review of Plan 0001, and a
candidate for Plan 0002's gate work).
