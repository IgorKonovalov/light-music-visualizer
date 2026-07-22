# 0008 — In-app preset browse overlay (standalone): on-canvas text + keyboard picker

> **Status:** approved
> **Created:** 2026-07-22
> **Owner skill(s):** dev
> **Related ADRs:** [ADR-0009](../adrs/0009-glyphon-text-rendering.md) — adopt glyphon for
> standalone on-canvas text, feature-gated (proposed by this plan; accepted at close). Builds on
> [ADR-0002](../adrs/0002-layered-preset-architecture.md) (the preset engine) and
> [ADR-0001](../adrs/0001-rust-core-wgpu-cabi-foobar-shim.md) (core owns raw GPU behind wgpu).
> Consumes Plan 0007's loading foundation.

## TL;DR

Plan 0007 made the preset library real and portable, but the only way to reach a preset is to
**cycle blind** (Space in the standalone, right-click Next in foobar) — a title-bar name after
the fact. This plan gives the standalone an **in-app browse overlay**: press a key, a scrollable
list of preset names appears over the running visual, arrow keys move a highlight, typing
narrows the list, Enter jumps straight to that preset, Esc closes. It needs the codebase's first
**text rendering** — adopted as **glyphon** behind a core `text` cargo feature
([ADR-0009](../adrs/0009-glyphon-text-rendering.md)), rendered through a small reusable
`render::text` seam so Plan 0009's live-show HUD can later draw through the same path. First
user-visible behavior lands in Phase 1: the standalone draws the **active preset's name on the
canvas** (not just the title bar). Standalone-only; the plugin stays cycle-only (no text over
the C ABI); selection is **keyboard-only** (projector/live-show friendly). The C ABI is
untouched.

## Context & problem

After Plan 0007 the standalone seeds and hot-reloads a per-user directory of ~10 curated presets
(growing as users add their own), and `Renderer` cycles them with `cycle_preset()`. Selection is
one-directional and nameless in the moment: an operator wanting a specific look taps Space
repeatedly and reads the title bar, which is unusable mid-show as the library grows past a
handful. The interview (2026-07-22) chose a **keyboard-driven browse overlay** with
**type-to-filter** as the standalone's real selection UX — the affordance Plan 0007 explicitly
split out because it requires a text stack.

Three things are missing today:

- **No text rendering at all.** The window title carries the preset name/FPS; nothing draws on
  the canvas. On-canvas text is a prerequisite for a list overlay and for the later HUD.
- **The renderer can only cycle, not address.** `Renderer` exposes `cycle_preset()` and
  `preset_name()`, but no way to *list* preset names or *select by index* — the overlay needs
  both.
- **No overlay input model.** The standalone's `winit` handler cycles on Space; there is no
  modal state (open/closed, highlight, filter string) driving a picker.

The user chose: **glyphon** for crisp scalable text (accepting its size cost, confined to the
standalone via a feature gate — [ADR-0009](../adrs/0009-glyphon-text-rendering.md)); a
**general** `render::text` seam reused by a future HUD, not a throwaway; **keyboard-only** input;
and a **flat list with type-to-filter** (no grouping/thumbnails — Plan 0007's "filenames only"
stance holds).

## Decision

Add a feature-gated `core::render::text` seam — a `TextLayer` (glyphon `FontSystem`/atlas/
`TextRenderer`) plus a per-frame queue of positioned text runs the frontend fills; `Renderer`
flushes it in a **second render pass** into the same surface view after the scene, before
present. Give `Renderer` preset **introspection + selection** (`preset_names()`,
`select_preset(index)`) beside the existing cycle. Build the overlay's modal logic as a **pure,
window-free `OverlayState`** in the standalone (open/closed, highlight, filter, key -> action),
unit-tested independently, and wire `winit` key events into it; each frame while open it emits
positioned text runs for the visible rows and, on Enter, calls `select_preset` with the
highlighted preset's **absolute** index. glyphon is enabled only for the standalone
(`features = ["text"]`); the plugin/default/core-test builds stay glyphon-free. We rejected a
bitmap atlas (blocky, chosen against), wgpu_text (no quality win), a standalone-owned GPU pass
(layer inversion — core owns the pass), and an unconditional glyphon dep (bloats the plugin) —
all recorded in ADR-0009.

## Architecture diagram

```mermaid
flowchart TD
    subgraph standalone [standalone/ — Rust, feature "text" on]
        keys[winit key events<br/>Tab / arrows / chars / Enter / Esc]
        ostate[OverlayState<br/>open? highlight, filter string<br/>pure, unit-tested]
        rows[visible rows -> TextRun list<br/>+ highlight]
        keys --> ostate
        ostate --> rows
    end

    subgraph core [core/ — render layer]
        names[Renderer::preset_names<br/>Renderer::select_preset idx]
        queue[per-frame text queue<br/>Vec of TextRun]
        text[render::text::TextLayer<br/>glyphon — feature-gated]
        pass["render(): scene pass -> text pass<br/>into one surface view"]
        queue --> text --> pass
    end

    ostate -- "Enter -> select_preset(absolute idx)" --> names
    rows -- "queue text each frame" --> queue
    names -. "names feed the list" .-> ostate
    hud[/"Plan 0009 HUD<br/>(later consumer)"/] -. reuses .-> queue
```

## Implementation phases

Each phase is one commit. `dev` runs all phases in one session; the architect reviews the whole
plan at the end.

### Phase 1 — glyphon dependency + `render::text` seam + on-canvas preset name (walking skeleton)

- **Owner skill:** dev
- **Area:** core (render), standalone
- **What:** Add glyphon to `core` behind a **non-default `text` feature**
  (`[features] text = ["dep:glyphon"]`, `glyphon` an optional exact-pinned dep), and add
  `core/src/render/text.rs` (carrying the hot-path panic pragma — it lives under `render/`, so
  the hygiene guard already requires it) with a `TextLayer` wrapping glyphon's
  `FontSystem`/`SwashCache`/atlas/`TextRenderer`/`Viewport` and a `TextRun { text, x, y, size,
  color }`. Give `Renderer` a per-frame text queue and a `queue_text(&[TextRun])`-style entry;
  in `render()`, after `scene.render(...)`, if the feature is on and the queue is non-empty,
  prepare + draw the runs in a second `RenderPass` (load, not clear) into the same `view`, then
  clear the queue. All glyphon-touching code sits under `#[cfg(feature = "text")]`; the
  default `render()` shape compiles unchanged. The standalone enables the feature and, each
  frame, queues the active preset name in a corner. glyphon is pinned to the release whose wgpu
  matches `=30.0.0` (verify at build — see Risks).
- **Files touched:** `core/Cargo.toml` (optional dep + feature), `core/src/render/text.rs`
  (new), `core/src/render/mod.rs` (queue + second pass, cfg-gated), `standalone/Cargo.toml`
  (`features = ["text"]`), `standalone/src/main.rs` (queue the active name each frame).
- **Done when:** `cargo run -p standalone` draws the active preset's name legibly on the canvas
  over the running visual (survives cycling and resize). `cargo tree -p lmv-core` (default
  features) shows **no glyphon**; `cargo tree -p standalone` shows it — a review-verifiable
  gate that the plugin/default build stays glyphon-free. Both `cargo build` (default) and
  `cargo build -p standalone` (feature on) compile. The standalone binary-size delta from
  glyphon is noted (on-box, NFR 4). glyphon↔wgpu-30 compatibility confirmed by a green build.

### Phase 2 — Renderer preset introspection + selection

- **Owner skill:** dev
- **Area:** core (render)
- **What:** Add `Renderer::preset_names(&self) -> impl Iterator<Item = &str>` (or `&[Preset]`
  accessor) returning the loaded presets in roster order, and
  `Renderer::select_preset(&mut self, index: usize)` that sets `active` to `index` **iff** it is
  in range (an out-of-range index is a no-op — never a panic, never wraps), returning the new
  active name for symmetry with `cycle_preset`. No feature gate needed — this is plain roster
  logic over the existing `presets` Vec. `set_presets` already clamps `active`; keep selection
  consistent with a hot-reload that shrinks the roster.
- **Files touched:** `core/src/render/mod.rs`, `core/tests/` (extend or add a render-roster
  test).
- **Done when:** a core unit/integration test builds a known multi-preset roster (via
  `set_presets`) and asserts `preset_names()` yields them in order, `select_preset(2)` makes
  `preset_name()` the third entry, and `select_preset(999)` leaves the active preset unchanged
  (no panic, no wrap) — the addressing contract, not "it compiles". `cargo nextest run` green.

### Phase 3 — Overlay state machine + keyboard wiring (open / move / select / close)

- **Owner skill:** dev
- **Area:** standalone
- **What:** Add a pure `OverlayState` in the standalone (own module) holding
  `open: bool`, `highlight: usize`, and (Phase 4) a filter string, with a
  `handle_key(key) -> OverlayAction` step where `OverlayAction` is one of `None`,
  `Redraw`, `Close`, or `Select(absolute_index)`. Tab/a chosen key toggles open; Up/Down move
  the highlight (clamped to the visible list); Enter emits `Select(absolute_index)`; Esc emits
  `Close`. Wire `winit`'s `KeyboardInput` into it: when the overlay is open it consumes the
  navigation keys; Space-cycle keeps working when **closed**. While open, the scene keeps
  rendering underneath and each frame the standalone queues the visible rows as `TextRun`s (with
  the highlighted row visually distinct) and a scroll window if the list exceeds the screen. On
  `Select`, call `Renderer::select_preset(index)`.
- **Files touched:** `standalone/src/overlay.rs` (new, with `#[cfg(test)]` unit tests),
  `standalone/src/main.rs` (declare the module, route keys, render rows while open).
- **Done when:** a standalone unit test drives `OverlayState` through open (Tab) -> Down, Down
  -> Enter and asserts it emits `Select` for the **third** preset's absolute index, reports the
  highlighted row along the way, and that Esc emits `Close`; keys while closed emit `None` so
  Space-cycle is unaffected. Runtime/visual (on-box): Tab opens a list over the scene, arrows
  move the highlight, Enter switches the visual, Esc closes.

### Phase 4 — Type-to-filter + hot-reload refresh + polish

- **Owner skill:** dev
- **Area:** standalone
- **What:** Extend `OverlayState` with an incremental **case-insensitive substring filter**:
  typed characters append to the filter and narrow the visible list; Backspace edits; the
  filtered view maps each visible row back to its **absolute** roster index so `Select` still
  addresses the right preset. Keep the highlight valid as the filter changes (clamp/reset to the
  first match). Integrate with hot-reload: when Plan 0007's ~500 ms poll swaps the roster, the
  overlay rebuilds its list from the current `preset_names()` without desyncing the highlight
  (re-clamp; keep open state). Small polish: an empty filter shows the whole list; a
  no-match filter shows an empty list, not a stale one.
- **Files touched:** `standalone/src/overlay.rs` (filter logic + tests), `standalone/src/main.rs`
  (feed typed chars; rebuild on reload).
- **Done when:** the `OverlayState` unit test is extended — typing `war` filters a known roster
  to the single matching name and Enter emits **its** absolute index; Backspace restores the
  broader list; the match is case-insensitive substring. Runtime/visual (on-box): typing narrows
  the on-screen list live, and editing a preset file while the overlay is open refreshes the list
  within ~1 s. `cargo nextest run` and clippy `-D warnings` stay green (default and
  `--features text` core builds both compile).

## Data shapes

```rust
// illustrative — not the final interface

// core::render::text — feature-gated seam, reused by the overlay and a later HUD.
#[cfg(feature = "text")]
pub struct TextRun<'a> {
    pub text: &'a str,
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub color: [f32; 4],
}

impl Renderer {
    pub fn preset_names(&self) -> impl Iterator<Item = &str>;
    /// Set the active preset iff `index` is in range (out-of-range is a no-op);
    /// returns the active preset name.
    pub fn select_preset(&mut self, index: usize) -> &str;
    /// Queue text runs to composite over the next rendered frame (cleared each
    /// frame). No-op when the `text` feature is off.
    pub fn queue_text(&mut self, runs: &[TextRun<'_>]);
}

// standalone::overlay — pure, window-free, unit-tested.
enum OverlayAction { None, Redraw, Close, Select(usize) /* absolute roster index */ }

struct OverlayState { open: bool, highlight: usize, filter: String }
impl OverlayState {
    fn handle_key(&mut self, key: Key, names: &[&str]) -> OverlayAction;
    fn visible<'a>(&self, names: &'a [&str]) -> Vec<(usize, &'a str)>; // (absolute idx, name)
}
```

## Risks & open questions

- **glyphon↔wgpu-30 compatibility is a hard gate (top risk).** glyphon tracks wgpu's major
  versions in lockstep; `dev` must pin the glyphon release whose wgpu dependency resolves to the
  workspace's `=30.0.0`. If **no** glyphon release targets wgpu 30, that is a wgpu-bump decision
  (its own ADR + a re-test of the whole render graph), **not** something to force here — stop and
  escalate at Phase 1 rather than bumping wgpu silently.
- **Dependency-tree / binary-size growth** (ADR-0009 negative). Accepted, and confined to the
  standalone by the `text` feature gate; measure the standalone binary before/after (NFR 4)
  and confirm glyphon is absent from the default/plugin dependency graph (`cargo tree`).
- **Two build shapes for `core::render`.** The `#[cfg(feature = "text")]` second pass means
  `render()` compiles two ways; the feature build must be exercised, not just the default. `dev`
  builds both locally this session; a `--features text` core check is a CI follow-up (note it
  at close, like the Miri/FFI CI gaps).
- **Overlap with Plan 0009 (live features), also standalone-only.** Both edit
  `standalone/src/main.rs` input handling, and Plan 0009's HUD is the second consumer of this
  plan's `render::text` seam. Whichever lands second rebases the `KeyboardInput` match arms and
  reuses the text queue rather than adding a parallel one — coordinate the sequencing; no new
  seam for the HUD.
- **Two text paths coexist by design (Plan 0011).** Plan 0011's diagnostics overlay uses a
  **core-side bitmap-digit** readout precisely so it reaches all three frontends (the plugin
  can't have glyphon — it's feature-gated out here). This plan's glyphon path is the
  standalone-only, quality-first text for browse/HUD. They are intentionally separate stacks, not
  duplication: plugin parity (bitmap digits) vs standalone polish (glyphon). If the browse
  overlay ever needs plugin parity, that is a re-decision, not a silent merge. Two consequences
  to keep straight: (1) this feature is named **`text`**, *not* `overlay`, so it does not read as
  gating Plan 0011's always-compiled diagnostics overlay (`render/overlay.rs`, `LMV_DEBUG_OVERLAY`);
  (2) both plans add a **conditionally-skipped final pass** to `Renderer::render` — Plan 0011's
  diagnostics pass and this plan's `text` pass. Whichever lands second **composes** (appends its
  pass after the scene, ordered) rather than replacing the other's; the diagnostics readout draws
  on top of the browse overlay when both are on.
- **Filter/selection index mapping.** The overlay must always `Select` the **absolute** roster
  index, not the filtered position — a unit test asserts this directly (Phase 4), since an
  off-by-one here silently selects the wrong preset.
- **Visual/runtime done-whens are on-box.** Phases 1/3/4's "legible on canvas", "switches the
  visual", "narrows live" are GPU/visual judgments, flagged like Plan 0003/0007's runtime
  done-whens; the pure `OverlayState` and renderer-selection tests are the review-verifiable
  core.

## What this plan does NOT do

- **No foobar overlay** — the plugin stays cycle-only (right-click Next). On-canvas text over the
  C ABI would be a far larger ABI change; out of scope, and the plugin has no keyboard-modal
  context.
- **No mouse** — keyboard-only, per the interview (projector/fullscreen live-show context).
- **No preset metadata / tags / grouping / thumbnails** — a flat list of names (Plan 0007's
  "filenames only" holds); grouping/preview was the interview's rejected richer option.
- **No HUD wiring** (FPS / system / live-show status on canvas) — that is Plan 0009 consuming the
  same `render::text` seam; this plan builds the seam and the browse overlay only.
- **No preset management from the overlay** — no rename/delete/reorder/favorite; browse + select
  only.
- **No new C ABI surface and no `LMV_ABI_VERSION` bump** — the overlay is entirely
  standalone/native-Rust; the plugin build is untouched.

## Followups (after this lands)

- **Plan 0009 — live performance features** reuses `render::text` for an on-canvas HUD (preset
  name / system / FPS / live-show status), retiring the title-bar surface.
- A `--features text` core build check in CI (the feature build's green is currently a local
  gate only), alongside the standing FFI/Miri CI notes.
- Mouse selection and/or preset metadata (tags, grouping, preview) if the flat keyboard list
  proves limiting once the library grows.
- Overlay affordances for preset management (favorite/reorder) — a later concern that interacts
  with metadata.
