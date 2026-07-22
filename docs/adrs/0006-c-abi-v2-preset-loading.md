# ADR-0006 — C ABI v2: add a preset-loading entry point

> **Status:** accepted
> **Date:** 2026-07-21 (accepted 2026-07-22 at Plan 0007 close)
> **Related plan(s):** [0007](../plans/0007-curated-preset-library.md)

## Context

ADR-0003 froze the v1 C ABI at exactly eight functions and named the one change that would
justify a v2: *"If scene selection later wants to be richer (pick-by-name, preset addressing
under ADR-0002's engine), that is a v2 ABI decision, not a silent edit."* Plan 0003 then built
ADR-0002's preset engine — the standalone loads TOML presets from a directory, seeds embedded
defaults, and hot-reloads. The plugin path was deferred: `lmv_cycle_scene` still cycles only the
core's *embedded* default presets, so a foobar user cannot reach the curated library or their own
authored presets. Plan 0007 closes that gap, which is exactly the preset-addressing v2 event
ADR-0003 anticipated.

Two forces shape the shape of the addition. First, the curated preset set is **embedded in
`lmv-core`** (`preset::EMBEDDED`, `include_str!`d at build) — the C++ shim has no access to those
bytes and no business duplicating them, so the *seeding* of a user directory with the curated set
must happen inside the core, reachable from C. Second, ADR-0003's whole point is that the
`extern "C"` surface stays minimal and every added function is a permanent compatibility
commitment compiled separately in C++ (drift is a link/runtime error, not a compile error). So the
addition must be the smallest surface that lets a host point the core at a directory and get the
curated-plus-user library rendering.

## Decision

We will add **one** function to the C ABI and bump `LMV_ABI_VERSION` from `1` to `2`:

> `int32_t lmv_load_presets(LmvHandle* handle, const uint8_t* path_utf8, size_t path_len);`

Its documented behavior is **seed-then-load**: given a UTF-8 directory path, the core (1) creates
the directory if absent and writes any missing embedded-curated preset file into it *without*
overwriting files already present, then (2) loads and installs every valid preset it finds,
replacing the handle's current preset set. It returns the number of presets installed (`>= 0`) or
a negative `LMV_ERR_*` code (null handle/path, invalid UTF-8, unusable directory). Consistent with
the rest of the ABI, it never panics across the boundary and a malformed preset directory degrades
to "keep the current set" rather than failing the call. The seed step is idempotent — safe to call
on every host start.

`lmv_cycle_scene` is unchanged and now cycles whatever set `lmv_load_presets` installed, exactly as
it already cycles the embedded defaults; selecting a preset *by name or index* over the ABI is **not**
part of this decision (the plugin's selection UX stays cycle-only; the standalone's in-app browser is
a separate, frontend-only plan). ADR-0003 remains the record of the v1 shape; this ADR extends it and
`LMV_ABI_VERSION = 2` is the new baseline any third consumer diffs against.

## Consequences

### Positive
- foobar reaches full preset parity with the standalone — the same curated library and the same
  user-editable directory — through one function, honoring ADR-0003's "minimal surface" ethos.
- Seeding lives in the core where the embedded curated bytes already are, so neither frontend
  duplicates preset content, and both can share one on-disk library directory.
- Adding `lmv_load_presets` finally gives the FFI a reason to grow an in-crate Rust test
  (create → load_presets on a temp dir → assert N installed), a first crack at the ABI's
  long-standing zero-CI-coverage gap (Plan 0001/0002 reviews).

### Negative (the price we pay)
- **The v2 surface is nine functions to keep stable forever.** Every addition is a permanent
  contract; this one in particular blesses a *directory-of-TOML* model of preset delivery. If
  preset delivery later wants to be different (a single-file bundle, an addressable catalog, an
  in-memory blob push), that is another ABI decision, not a silent reshape.
- **`lmv_load_presets` has a write side effect** (seeding), which is mild least-surprise debt for a
  function whose name says "load." It is documented, idempotent, and never overwrites user files —
  but a host calling it against a read-only path must tolerate the seed step no-op'ing and getting
  only whatever already loads. The two-function alternative below trades this away for a wider
  surface; we judged the smaller surface the better deal.

### Neutral
- The header stays hand-synced with `ffi.rs` (no cbindgen yet, per ADR-0003), so the new signature
  and its error codes are a review/link-time contract like the other eight.
- Paths cross the ABI as UTF-8 bytes + length (not a null-terminated `char*`), matching how the host
  already passes buffers; a non-UTF-8 path is rejected rather than best-effort decoded.

## Alternatives considered

### Alternative A — Two functions: `lmv_seed_presets` + `lmv_load_presets`
Split the write (seed the curated set) from the read (load + install), giving each a single
responsibility and removing the "load also writes" surprise. Rejected: it widens the frozen surface
to ten functions for a bootstrap step that every host performs immediately before loading anyway,
and ADR-0003's governing value is surface minimality over per-call purity. One documented,
idempotent seed-then-load call is the better trade at this size.

### Alternative B — Load-only ABI; each frontend seeds the curated set itself
Keep `lmv_load_presets` a pure read and let each frontend write the curated files first. Rejected:
the curated bytes are embedded in `lmv-core`; the C++ shim would either need a *second* ABI function
to fetch them or a hand-maintained copy of every preset in C++ — reintroducing exactly the content
duplication and drift the embedded set exists to prevent.

### Alternative C — Don't touch the ABI; foobar keeps the embedded defaults
Leave v1 alone and accept that the plugin renders only the built-in defaults. Rejected: plugin
preset parity is the explicit goal of Plan 0007 (and a stated project value — the plugin path is
why parity matters on macOS), and ADR-0003 already earmarked preset addressing as the v2 trigger.
Deferring only defers the same bump.

## Notes

Supersedes nothing; extends ADR-0003 (v1 shape stays the historical record). The v1→v2 handshake
still runs through `lmv_abi_version`, so a plugin built against v1 and loaded on a v2 core (or the
reverse) detects the mismatch instead of calling a function that isn't there.
