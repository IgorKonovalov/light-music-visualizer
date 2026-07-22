//! Convention guard tests (Plan 0002 Phase 2). std-only, no dependency, so
//! "lightweight is a feature" holds even for the guardrails.
//!
//! (a) Every hot-path module carries the panic-denial pragma, so a newly
//!     added hot module can't silently ship without it.
//! (b) Every direct dependency in a workspace member manifest is exact-pinned
//!     (`=x.y.z`), per CLAUDE.md ("pin direct dependencies to exact versions").

use std::path::{Path, PathBuf};

/// The panic-denial header every hot-path module must carry. Copy it verbatim
/// to the top of any new file under `core/src/dsp/`, `core/src/render/`,
/// `core/src/diag/`, `core/src/ffi.rs`, `core/src/audio.rs`,
/// `core/src/preset/expr.rs`, or the `lmv-ring` crate's `src/` (the extracted
/// SPSC ring, Plan 0005):
///
/// ```ignore
/// #![deny(
///     clippy::unwrap_used,
///     clippy::expect_used,
///     clippy::indexing_slicing,
///     clippy::panic,
///     clippy::unreachable
/// )]
/// ```
///
/// `indexing_slicing` is the grep-able sentinel proving the block is present.
const PRAGMA_SENTINEL: &str = "clippy::indexing_slicing";

fn core_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// The workspace root — the parent of the `core` crate this test lives in.
/// Used to reach sibling crates (`lmv-ring`, `standalone`) whose manifests and
/// hot-path source the guards below also cover.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("core crate has a workspace-root parent")
        .to_path_buf()
}

fn collect_rs_files(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_file() {
        if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path.to_path_buf());
        }
        return;
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(path)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", path.display()))
        .map(|entry| entry.expect("dir entry").path())
        .collect();
    entries.sort();
    for entry in &entries {
        collect_rs_files(entry, out);
    }
}

/// The hot-path set the pragma guards. Directories are scanned recursively;
/// a new hot-path directory added by a later plan must be listed here (a
/// Mode 4 review item — see the plan's Design-integrity note).
#[test]
fn hot_path_modules_carry_the_panic_pragma() {
    let src = core_src();
    let targets = [
        src.join("dsp"),
        src.join("render"),
        src.join("diag"),
        src.join("ffi.rs"),
        src.join("audio.rs"),
        // Per-frame preset evaluator (Plan 0003): a single hot-path file inside
        // an otherwise load-time module, so it is listed directly rather than
        // scanning all of `src/preset/`.
        src.join("preset").join("expr.rs"),
        // The SPSC ring's `unsafe` now lives in the sibling lmv-ring crate
        // (Plan 0005); its whole `src/` is hot-path code.
        workspace_root().join("lmv-ring").join("src"),
    ];

    let mut files = Vec::new();
    for target in &targets {
        assert!(
            target.exists(),
            "hot-path target is missing: {}",
            target.display()
        );
        collect_rs_files(target, &mut files);
    }
    assert!(!files.is_empty(), "found no hot-path source files to check");

    for file in &files {
        let text = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("read {}: {e}", file.display()));
        assert!(
            text.contains(PRAGMA_SENTINEL),
            "hot-path module `{}` is missing the panic-denial pragma \
             (sentinel `{PRAGMA_SENTINEL}`). Copy the `#![deny(...)]` block \
             from tests/hygiene.rs to the top of the file.",
            file.display(),
        );
    }
}

#[test]
fn direct_dependencies_are_exact_pinned() {
    let root = workspace_root();
    let manifests = [
        root.join("core").join("Cargo.toml"),
        root.join("lmv-ring").join("Cargo.toml"),
        root.join("standalone").join("Cargo.toml"),
    ];
    for manifest in &manifests {
        check_exact_pins(manifest);
    }
}

fn check_exact_pins(manifest: &Path) {
    let text = std::fs::read_to_string(manifest)
        .unwrap_or_else(|e| panic!("read {}: {e}", manifest.display()));

    let mut in_deps = false;
    let mut depth: i32 = 0;

    for raw in text.lines() {
        let line = raw.trim();

        // Section headers only register at the top level (depth 0).
        if depth == 0 && line.starts_with('[') {
            in_deps = is_dependency_header(line);
            continue;
        }

        // Parse entries only at the top level of a deps table; interior lines
        // of a multi-line inline table (depth > 0) are array/table members.
        if in_deps
            && depth == 0
            && let Some((name, value)) = dependency_entry(line)
            && let Some(version) = declared_version(value)
        {
            assert!(
                version.starts_with('='),
                "{}: dependency `{name}` is not exact-pinned (found `{version}`); \
                 use `=x.y.z` (CLAUDE.md).",
                manifest.display(),
            );
        }

        depth += bracket_delta(line);
        if depth < 0 {
            depth = 0;
        }
    }
}

/// Headers ending in `dependencies]` cover `[dependencies]`,
/// `[build-dependencies]`, and the per-target `[target.'...'.dependencies]`
/// tables where the standalone's real deps live.
fn is_dependency_header(line: &str) -> bool {
    line.starts_with('[') && line.ends_with("dependencies]")
}

/// Net change in `{`/`[` nesting on a line (parentheses ignored).
fn bracket_delta(line: &str) -> i32 {
    line.chars().fold(0, |acc, c| match c {
        '{' | '[' => acc + 1,
        '}' | ']' => acc - 1,
        _ => acc,
    })
}

/// A `name = value` dependency line, or `None` for blanks/comments/non-entry
/// lines. `name` must be a bare dependency identifier.
fn dependency_entry(line: &str) -> Option<(&str, &str)> {
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let (name, value) = line.split_once('=')?;
    let name = name.trim();
    let is_ident = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if is_ident {
        Some((name, value.trim()))
    } else {
        None
    }
}

/// The version requirement a dependency value declares, or `None` for
/// `path`/`workspace` deps that carry no version.
fn declared_version(value: &str) -> Option<String> {
    let value = value.trim();
    if value.starts_with('"') {
        return first_quoted(value).map(str::to_string);
    }
    if value.starts_with('{') {
        let key = value.find("version")?;
        let after = value.get(key + "version".len()..)?.trim_start();
        let after = after.strip_prefix('=')?.trim_start();
        return first_quoted(after).map(str::to_string);
    }
    None
}

fn first_quoted(s: &str) -> Option<&str> {
    let start = s.find('"')?;
    let rest = s.get(start + 1..)?;
    let end = rest.find('"')?;
    rest.get(..end)
}
