# Commit conventions — dev

Conventional Commits, one logical change (or one plan phase) per commit. There is no commitizen
hook in this repo yet; the convention is enforced by discipline and the architect's review.

## Format

```
<type>(<scope>): <subject>

[optional body — wrap at ~72 chars, plain ASCII]

[optional footer — references, BREAKING CHANGE]
```

- **type**: from the table below.
- **scope**: the crate/area touched. Optional but encouraged — it scans the log far better.
- **subject**: imperative, lowercase, no trailing period, ≤ ~72 chars.

Commit the message via the **PowerShell tool's single-quoted here-string** (`@'...'@`, closing
`'@` at column 0). Keep the body plain ASCII — straight hyphens, no em-dashes, no internal
double-quotes — or git may misparse the here-string into stray pathspecs.

## Types

| Type       | When to use |
|------------|-------------|
| `feat`     | A new user-visible capability — a new scene, a new DSP output, a new C ABI function, capture working on a platform. |
| `fix`      | A bug fix in existing code. |
| `refactor` | Internal restructuring, no behavior change. |
| `perf`     | A change made specifically for real-time/throughput/latency. |
| `test`     | Adding or fixing tests. (Tests for new code in the same phase fold into that `feat` commit.) |
| `docs`     | Markdown, doc comments, ADRs, plans, READMEs. |
| `chore`    | Misc maintenance — file moves, `.gitignore`, tooling config. |
| `build`    | Build-system / dependency changes (`Cargo.toml`, lockfile, the plugin build project). |
| `ci`       | CI config under `.github/workflows/`. |

## Scopes

Smallest meaningful scope. Omit if a commit truly spans many (usually a sign to split).

| Scope        | Area |
|--------------|------|
| `core`       | `core/` generally |
| `audio`      | `core/src/audio.rs` — ring buffer, sample intake |
| `dsp`        | `core/src/dsp/` — FFT, onset, beat |
| `render`     | `core/src/render/` — wgpu layer |
| `scenes`     | `core/src/scenes/` |
| `ffi`        | `core/src/ffi.rs`, `core/include/` — the C ABI |
| `standalone` | `standalone/` — winit, capture, input |
| `plugin`     | `plugin-foobar/` — C++ shim |
| `tooling`    | `Cargo.toml`, lockfile, rust-toolchain, `.gitignore` |
| `ci`         | `.github/workflows/` |
| `docs`       | anything under `docs/` |

## Examples

```
feat(dsp): windowed FFT producing normalized log-frequency spectrum
```

```
feat(audio): lock-free SPSC ring buffer for source-agnostic sample intake

Validates sample rate and channel count once at push_samples; the
downstream DSP path trusts them. No allocation on the producer side.
```

```
perf(audio): drop the mutex in the WASAPI callback for a lock-free ring

The capture callback must never block. Replaces the Mutex<VecDeque>
with an SPSC ring so the audio thread only does a memcpy and returns.
```

```
feat(ffi): expose lmv_create / lmv_push_samples / lmv_render / lmv_free

Minimal versioned C ABI. Panics are caught at the boundary and mapped
to error codes so a fault never crosses FFI into the C++ host as UB.
```

## When to split

Default to splitting when a phase has logically independent pieces — the architect's review is
easier when each commit tells one story. Good split for a DSP phase:

1. `feat(dsp): add fft module with windowing`
2. `feat(dsp): add onset/beat estimator`
3. `test(dsp): cover sine-to-single-bin and click-track onset detection`

Bad: a single opaque `feat: implement phase 2`.

## When NOT to split

- Tests tightly coupled to new code ship in the same `feat` commit.
- Mechanical cross-file renames go in one commit.
- A bugfix plus its regression test go in one `fix` commit.

## Every commit must NOT contain

- Secrets, signing/notarization credentials — not in the diff, message, or body.
- `--no-verify` shortcuts, or `#[allow(...)]` added only to dodge a real clippy warning.
- Co-author trailers, unless the user explicitly asks.
- Broad staging (`git add -A` / `.`). Name files explicitly; you own what enters the index.
