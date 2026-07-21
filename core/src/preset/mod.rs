//! The preset layer — ADR-0002 layers 1-2: TOML data binding built-in system
//! parameters to a pure expression language over the audio analysis.
//!
//! [`expr`] compiles and evaluates expression strings; [`schema`] parses a
//! TOML preset into compiled [`Binding`]s. This module also loads presets in
//! bulk: [`default_presets`] embeds the shipped examples (so the C-ABI/foobar
//! path always has visuals without a preset directory), [`seed_dir`] writes the
//! embedded curated set into a per-user directory on first run (write-if-absent,
//! so a user's edits survive), and [`load_dir`] reads a directory for the
//! standalone's hot-reload path — a malformed file is reported, never fatal, so
//! the caller keeps the last good set (NFR 10).

pub mod expr;
pub mod schema;

use std::path::{Path, PathBuf};

pub use expr::{Expr, ExprError, Variables, compile};
pub use schema::{Binding, Preset, PresetError, SystemKind};

/// The shipped example presets, embedded at compile time. These are the exact
/// files under `presets/` at the repo root, so the embedded defaults and the
/// on-disk hot-reload source never drift.
const EMBEDDED: [(&str, &str); 4] = [
    (
        "fragment_aurora.toml",
        include_str!("../../../presets/fragment_aurora.toml"),
    ),
    (
        "fragment_pulse.toml",
        include_str!("../../../presets/fragment_pulse.toml"),
    ),
    (
        "swarm_flow.toml",
        include_str!("../../../presets/swarm_flow.toml"),
    ),
    (
        "swarm_burst.toml",
        include_str!("../../../presets/swarm_burst.toml"),
    ),
];

/// Parse the embedded example presets. The shipped files are valid, so on the
/// off chance one fails it is skipped rather than panicking — the caller still
/// gets a usable set.
pub fn default_presets() -> Vec<Preset> {
    EMBEDDED
        .iter()
        .filter_map(|(_, src)| Preset::from_toml_str(src).ok())
        .collect()
}

/// Write each embedded curated preset into `dir`, creating `dir` (and any
/// missing parents) first, but **never overwriting** a file that already
/// exists — a user's edits to a seeded preset survive re-seeding. Returns how
/// many files were newly written. Idempotent: a second call on an
/// already-seeded directory writes zero.
///
/// Because seeding never clobbers, a curated preset changed in a later release
/// does **not** replace the copy a user already has on disk (a "refresh
/// curated" affordance is a follow-up, not this function's job). Errors bubble
/// up as `io::Result` so the caller can degrade to the embedded defaults rather
/// than crash (NFR 10).
pub fn seed_dir(dir: &Path) -> std::io::Result<usize> {
    std::fs::create_dir_all(dir)?;
    let mut written = 0;
    for (name, contents) in EMBEDDED {
        let path = dir.join(name);
        if !path.exists() {
            std::fs::write(&path, contents)?;
            written += 1;
        }
    }
    Ok(written)
}

/// The outcome of loading a preset directory: the presets that compiled, in
/// filename order, plus the files that failed (so the caller can surface them).
pub struct LoadReport {
    /// Successfully compiled presets, sorted by filename for a stable cycle.
    pub presets: Vec<Preset>,
    /// `(path, error)` for each `.toml` that failed to read or compile.
    pub errors: Vec<(PathBuf, PresetError)>,
}

/// Load every `*.toml` in `dir`, compiling each into a [`Preset`]. Missing or
/// unreadable directories yield an empty report rather than an error; a bad
/// file lands in `errors` and does not stop the others (degrade, never crash).
pub fn load_dir(dir: &Path) -> LoadReport {
    let mut presets = Vec::new();
    let mut errors = Vec::new();

    let mut paths: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(entries) => entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|ext| ext == "toml"))
            .collect(),
        Err(_) => return LoadReport { presets, errors },
    };
    paths.sort();

    for path in paths {
        match std::fs::read_to_string(&path) {
            Ok(src) => match Preset::from_toml_str(&src) {
                Ok(preset) => presets.push(preset),
                Err(err) => errors.push((path, err)),
            },
            Err(err) => errors.push((path, PresetError::Io(err.to_string()))),
        }
    }

    LoadReport { presets, errors }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_dir_writes_all_then_nothing() {
        let dir = std::env::temp_dir().join("lmv_seed_dir_test");
        let _ = std::fs::remove_dir_all(&dir);

        // First seed into an empty dir: every embedded preset is written.
        let written = seed_dir(&dir).expect("seed into fresh temp dir");
        assert_eq!(
            written,
            EMBEDDED.len(),
            "first seed writes every embedded preset"
        );
        for (name, _) in EMBEDDED {
            assert!(dir.join(name).exists(), "{name} was seeded");
        }

        // Second seed: write-if-absent means nothing is written and nothing is
        // clobbered.
        let again = seed_dir(&dir).expect("re-seed already-seeded dir");
        assert_eq!(
            again, 0,
            "re-seeding writes zero (idempotent, no overwrite)"
        );

        // Deleting one seeded file re-seeds only that file.
        let (victim, _) = EMBEDDED[0];
        std::fs::remove_file(dir.join(victim)).expect("remove one seeded file");
        let refill = seed_dir(&dir).expect("re-seed after deletion");
        assert_eq!(refill, 1, "only the missing file is re-written");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
