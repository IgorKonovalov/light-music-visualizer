# ADR-0002 — Layered preset architecture: data + expressions + optional script

**Status:** accepted (2026-07-21) — implemented by [Plan 0003](../plans/done/0003-generative-scenes-and-presets.md),
which built **layers 1-2**: TOML data presets, the pure expression language, and the first two
built-in systems (fragment field + ~10k CPU swarm). Layer 3 (Rhai orchestration), cross-preset
blending, compute-scale particles, and the remaining built-in systems (feedback/warp, boids,
walkers, 3D) stay deferred follow-ups.

## Context

The primary use case is live DJ shows (NFR §10): scenes rotate across a multi-hour mix, and
the user wants visualizations authored as **lightweight, MilkDrop-akin text presets** rather
than Rust code — including staged, coherent arcs within a track (intro → build → drop) and
generative-art systems (walkers, flocks, 3D renders).

Three forces constrain the design:

- **The performance floor** (NFR §1): 60 fps at 1080p on an integrated GPU. Per-entity
  simulation in an interpreted scripting language cannot hold that floor at interesting
  entity counts.
- **Lightweight** (NFR §4): ~10 MB soft cap; no heavyweight runtime or C++ dependency.
- **Live reliability** (NFR §10): a misbehaving preset must degrade, never crash or stall a
  show.

## Decision

Presets are **three layers, each optional above the first**:

1. **Data layer (the preset file, TOML).** Declares composed **built-in systems** and binds
   their parameters to **expression strings** — a small, pure expression language over the
   audio/analysis variables (`bass`, `mid`, `treb`, `onset`, `beat`, `bar`, `time`, ...).
   Most presets are only this. Expressions are pure and allocation-free per evaluation.
2. **Built-in generative systems (Rust + wgpu, the rendering vocabulary).** First-class
   v1 set: **feedback/warp field**, **flocking (boids)**, **random walkers / growth**, and a
   **3D scene system** (instanced meshes, audio-driven camera). All per-entity simulation
   lives here (CPU for hundreds, compute shaders for thousands) — never in script.
3. **Optional behavior layer (Rhai script).** A preset may attach a script with lifecycle
   hooks (`on_load`, `on_beat`, `on_phase`, `on_frame`) that **orchestrates** — advances
   phases, tweens parameters, spawns/retires layers — to stage a coherent per-track arc.
   Rhai is a pure-Rust embeddable language: sandboxed, no C dependency. Script execution is
   **budgeted per frame**; a preset that blows its budget is throttled or reloaded, not
   allowed to stall the render loop.

Presets hot-reload from disk, and the engine blends between presets on scene change (both
are engine features the plan details; the ADR fixes only the authoring model).

## Consequences

**Positive**

- Authoring stays approachable and text-based (MilkDrop's virtue); complexity is opt-in —
  data-only presets are trivial, scripts only where an arc needs logic.
- The perf floor is structurally protected: hot per-entity math is Rust/GPU; scripts touch
  parameters, not particles.
- Sandboxed by construction — expressions are pure, Rhai is embedded and budgeted; a bad
  preset cannot take down a live show.
- Pure-Rust stack (Rhai, TOML, expression evaluator) keeps the size cap and build simplicity.

**Negative (the price we pay)**

- **We own two languages:** the expression grammar and the Rhai host API. Both become
  compatibility surfaces — presets written today must keep working; the host API needs
  versioning discipline like the C ABI.
- **Novelty is bounded by the built-in vocabulary.** A preset cannot invent a system we
  didn't ship. If that bites, the escape hatch is an author-supplied WGSL pass layer —
  deliberately deferred to a future ADR, not smuggled in.
- The built-in systems (especially 3D) are a real engineering investment before the first
  community-visible payoff.

## Alternatives considered

- **Data-only presets (no scripting).** Simplest and fully sandboxed, but declarative phase
  tables cannot express custom staged logic — the "coherent scene within a track" requirement
  is exactly what dies. Rejected; the data layer survives as the foundation.
- **Shader-centric presets (author-written WGSL compute + fragment passes).** Closest to
  MilkDrop's soul and the fastest ceiling, but the authoring bar is shader literacy, and
  every generative system (walkers, boids) must be hand-written as a compute shader —
  the steepest path to the stated generative-art goal. Rejected as the *authoring* model;
  may return later as an advanced pass type via a new ADR.
- **Script-owns-the-scene (Lua/Rhai draws everything per frame).** Maximum flexibility;
  rejected because CPU-side per-entity scripting cannot hold 60 fps on the iGPU baseline,
  and the drawing API surface would dwarf the C ABI in maintenance cost.
- **MilkDrop `.milk` compatibility (projectM or a reimplementation).** Inherits thousands of
  existing presets, but means a heavyweight C++ dependency (or a massive reimplementation),
  contradicting the Rust core, the size cap, and the lightweight goal. Rejected.
- **Lua (mlua) instead of Rhai for the behavior layer.** Mature and fast, but binds a C
  library into an otherwise pure-Rust core. Rhai's pure-Rust embedding wins at our script
  sizes (orchestration, not simulation). Revisit only if profiling shows Rhai overhead
  matters — which the "no simulation in script" rule is designed to prevent.
