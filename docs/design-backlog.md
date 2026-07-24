# Design backlog — captured feedback, not yet promoted

Short, durable notes for design gaps surfaced during work but **not yet** decided into an ADR or
plan. Chiefly the `preset-author → architect` feedback handoff (a look wanting something the
preset grammar or engine can't express), plus any other "worth remembering, not worth acting on
yet" finding.

An entry here is **not** a commitment to build — it is a captured signal so the friction isn't
lost between sessions. When one is acted on, it graduates to an ADR (if it has a real rejected
alternative) and/or a plan, and the entry is struck through with a pointer to where it went.
Verify every entry against the code before acting on it — these are dated snapshots, and the
surface moves (same rule the lanes apply to their own references).

---

## 0001 — reaction_diffusion reaches only 2 of the 5 Plan-0018 composite levers

- **Raised:** 2026-07-24, from `preset-author` (authoring the "Chthonic Coral Oracle" coral preset).
- **Verified against code:** yes — see the per-lever notes below.
- **PROMOTED 2026-07-24 → [ADR-0026](adrs/0026-full-composite-coverage-fullscreen-scenes.md) +
  [Plan 0025](plans/0025-full-composite-coverage.md)** (full-audit scope: background + view transform
  for reaction-diffusion *and* attractor, via alpha-present-over-backdrop). Notes retained below as the
  origin record.

Plan 0018 shipped five engine-wide, audio-bindable composite controls (view zoom/pan, background
atmosphere, geometry mirror, feedback trails, screen-space kaleidoscope). The **reaction_diffusion**
(Gray-Scott coral) scene participates in only two of them. A preset author composing the coral
scene silently loses three families of named params — they parse fine (no `deny_unknown_fields`)
but do nothing.

| Lever | Params | Reaches RD? | Why |
|-------|--------|-------------|-----|
| Feedback trails | `trails` | **yes** | Post-pass over the composited frame (`render/trails.rs`) — scene-agnostic. |
| Screen-space kaleidoscope | `kaleido_order`, `kaleido_angle` | **yes** | Post-pass over the offscreen frame (`render/kaleidoscope.rs`) — scene-agnostic. |
| Background / atmosphere | `bg_hue`, `bg_bright`, `bg_vignette` | **no** | The `bg_*` pre-pass draws first, but RD's present is a **fullscreen opaque** pass (`reaction_diffusion.rs::render`, `LoadOp::Load` + `BlendState::REPLACE`, alpha 1 everywhere) — it overwrites the backdrop. |
| View transform | `zoom`, `pan_x`, `pan_y` | **no** | `ViewTransform` is consumed only by `fragment_field`, `swarm`, and the line scenes. RD's `render` takes no transform; its field samples 1:1 to screen. |
| Geometry mirror | `mirror_order`, `mirror_reflect` | **N/A by nature** | Line-segment replication before the segment cap — a fullscreen field has no segments. The screen-space kaleidoscope is the right tool for RD's symmetry and already works; **not a gap, just a clarification.** |

**So there are two genuine gaps (background, view transform) and one non-gap (geometry mirror).**

- **Highest value: background compositing.** The coral look is mostly dark — black voids between the
  contours. If RD composited over the `bg_*` atmosphere instead of overwriting it (present with
  alpha/blend so `V≈0` reads as transparent, or an explicit backdrop-aware present), those voids
  would fill with the tintable gradient. Big aesthetic upside for a small preset author, and the
  thing most likely to make the coral scene feel "finished."
- **Lower value: view zoom/pan on RD.** Wiring the `ViewTransform` into the present pass's sample UVs
  would let a preset zoom into the reef. Straightforward but less impactful than the backdrop, since
  the kaleidoscope already supplies large-scale motion.

**Likely also affected: the `attractor` scene** (also absent from the `ViewTransform` consumer list).
Confirm its lever coverage if this is promoted — the fix may want to be "audit composite coverage
across *all* scenes," not RD alone.

**ADR-worthy if pursued.** Touches ADR-0018's fixed-order composite (where/how a fullscreen scene
hands off to the background) and the `Scene` render seam (does every scene take a `ViewTransform`, or
does the composite own it?). The rejected alternative — "leave RD opaque; backgrounds are for the
line/particle scenes only" — is nameable, so a decision here earns an ADR, then a small plan.
