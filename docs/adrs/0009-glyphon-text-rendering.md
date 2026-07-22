# ADR-0009 — Adopt glyphon for standalone on-canvas text (feature-gated)

> **Status:** accepted
> **Date:** 2026-07-22
> **Related plan(s):** [0008](../plans/0008-preset-browse-overlay.md) — the in-app preset
> browse overlay (this ADR's first consumer). Extends [ADR-0001](0001-rust-core-wgpu-cabi-foobar-shim.md)
> (core owns all raw GPU behind wgpu) and respects the NFR 4 dependency/size cap.

## Context

The standalone needs on-canvas text. First for the Plan 0008 preset-browse overlay — a
scrollable, type-to-filterable list of preset names drawn over the running visual — and later
for a live-show HUD (Plan 0009: preset name / system / FPS / status), which today live only in
the window title bar. The codebase has no text rendering of any kind.

Two forces shape the decision. First, **compositing**: text must draw over the wgpu-rendered
scene in the *same* frame, so it has to render through the `wgpu::Device`/`Queue`/surface the
core's `RenderContext` owns. A frontend cannot cleanly run its own glyph pass without core
handing out its device and its in-flight render pass — which would invert ADR-0001's rule that
the core owns all raw GPU and the frontend never sees a backend. Second, **cost**: "lightweight
is a feature" (NFR 4), and the overlay is *standalone-only* — the foobar plugin's selection
stays cycle-only, so the plugin's C-ABI build must not carry a text stack it can never reach.

The user chose (interview, 2026-07-22) crisp, scalable anti-aliased text over a blocky
hand-rolled bitmap atlas, and asked that the text seam be general enough that Plan 0009's HUD
reuses it rather than a throwaway.

## Decision

We will adopt **glyphon** (cosmic-text + swash on wgpu) as the standalone's text renderer,
living in the core's render layer **behind a non-default cargo feature `text`**. The
standalone enables `lmv-core = { path = "../core", features = ["text"] }`; the plugin's
`cdylib`/`staticlib` build, the default `cargo build`, and the core test suite compile glyphon
out entirely.

A small `render::text` seam — a `TextLayer` wrapping glyphon's `FontSystem`, atlas, and
`TextRenderer`, plus a per-frame queue of positioned text runs — draws in a second render pass
into the same surface view *after* the scene and *before* present, so text composites over the
visual in one frame. Both the overlay and a future HUD consume that one seam. glyphon is pinned
to the **exact release whose wgpu dependency matches the workspace's `=30.0.0`**, a hard
compatibility constraint verified at adoption; if no glyphon release targets wgpu 30, that is a
separate wgpu-bump decision (its own ADR), not something silently resolved here.

The C ABI is **untouched** — no version bump. The plugin gains nothing here, by design.

## Consequences

### Positive
- Crisp, scalable, Unicode-capable text for the overlay and the later HUD through one reusable
  core seam — no throwaway, matching the user's "build it general" choice.
- Text composites correctly over the scene because it renders through core's own
  device/queue/pass; ADR-0001's layering (core owns raw GPU, the frontend sees none) is
  preserved.
- The plugin and the default build stay glyphon-free, so the accepted dependency-tree/size cost
  is confined to the one frontend the user opted into.

### Negative (the price we pay)
- glyphon pulls a **sizable transitive tree** (cosmic-text, swash, ...) — a real hit to the
  standalone's binary size and to the exact-pin surface. It is the first dependency accepted
  primarily for UI polish rather than a core capability.
- Feature-gating adds `#[cfg(feature = "text")]` seams to the render hot path (a second pass
  in `render()`), so `core::render` has **two compiled shapes** to keep working — a small
  readability and build-matrix cost (the feature build must be exercised, not just the default).
- glyphon↔wgpu **version lockstep** couples our wgpu upgrades to glyphon's release cadence; a
  future wgpu bump may be gated on glyphon catching up.

### Neutral
- The text seam lives in `core`, not `standalone`, even though only the standalone uses it
  today — because that is where the GPU context is. The **feature gate**, not a crate boundary,
  is what keeps it out of the plugin.
- `render::text` is under `core/src/render/`, so the Plan 0002 hygiene guard's recursive scan
  already requires the panic-denial pragma on it — the new hot-path file joins the guard for
  free.

## Alternatives considered

### Alternative A — Embedded bitmap-font atlas (zero new dependency)
An ASCII monospace glyph atlas embedded via `include_bytes!`, drawn as textured quads. Rejected:
the user chose crisp scalable text; a blocky atlas upscaled on a projector for a live show reads
poorly, and hand-rolling layout plus a filter cursor is its own non-trivial cost for a worse
result.

### Alternative B — wgpu_text / glyph_brush (ab_glyph, lighter than glyphon)
A middle-ground TTF glyph cache. Rejected: it is still a new dependency tree with no quality win
over glyphon's shaping and anti-aliasing, and it caps capability for the later HUD; the savings
don't justify a second-best text stack.

### Alternative C — Text in a standalone-owned pass, exposing core's device/queue/view
Let the standalone own glyphon and render a second pass by having core hand back its device,
queue, and the acquired surface view. Rejected: it leaks the GPU context and the in-flight
render pass out of core, inverting ADR-0001 (the frontend would drive raw wgpu against core's
surface texture) — a layer inversion for no gain over a core-side seam.

### Alternative D — Add glyphon to core unconditionally (no feature gate)
Simplest wiring. Rejected: it bloats the plugin/C-ABI build with a text stack it can never reach
(plugin selection is cycle-only), violating "lightweight is a feature" for the frontend most
sensitive to size. The gate is cheap insurance against that.

## Notes

Supersedes nothing. The `text` feature is the seam's on/off switch. It is named `text` (for the
`render::text` seam), deliberately **not** `overlay`, to avoid colliding with the concurrent
Plan 0011 / [ADR-0008](0008-c-abi-v3-diagnostics.md) diagnostics overlay (`render/overlay.rs`,
the `LMV_DEBUG_OVERLAY` flag) — that overlay is always compiled, dependency-free, and paints in
all three frontends, so it is emphatically *not* behind this feature. Because the C ABI is
unchanged, `LMV_ABI_VERSION` stays at 2 and the plugin build is unaffected.
