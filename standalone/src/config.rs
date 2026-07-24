//! Per-user operator config for the live-show standalone (Plan 0009).
//!
//! A small `config.toml` under the same per-user app dir the presets live in
//! (`%APPDATA%\light-music-visualizer\` on Windows). Read once at startup and
//! written back whenever a hotkey changes a choice, so a stage setup survives a
//! restart. Only the fields the live-show features need — the full
//! settings-persistence UX stays a later roadmap item.
//!
//! Every field is `#[serde(default)]`, so a missing file, a missing section, or
//! an unknown extra key all degrade to the built-in defaults rather than crash
//! (NFR section 10 "degrade, never crash"). Later phases grow this schema
//! (`[input]`, `[rotate]`); keep additions default-able for the same reason.

use std::path::Path;

use serde::{Deserialize, Serialize};

/// The whole operator config. `#[serde(default)]` on the container fills in any
/// section the file omits.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub output: Output,
    pub input: Input,
    pub rotate: Rotate,
}

/// `[input]` — where audio comes from: loopback of whatever is playing, or a
/// line-in / audio-interface capture device (Plan 0009 Phase 2, Windows-first).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Input {
    /// Loopback of a render device, or direct capture of an input device.
    pub mode: InputMode,
    /// Friendly device name to capture. `"default"` (or a name that matches no
    /// active endpoint) falls back to the default endpoint of the selected
    /// mode's dataflow.
    pub device: String,
}

impl Default for Input {
    fn default() -> Self {
        // Loopback of the default render device — the pre-Plan-0009 behavior, so
        // an existing user with no `[input]` section keeps what they had.
        Self {
            mode: InputMode::Loopback,
            device: "default".to_owned(),
        }
    }
}

/// The capture path. Serializes as the kebab-case strings the config uses
/// (`"loopback"` / `"line-in"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InputMode {
    /// Tap a render device (what the system is playing).
    #[default]
    Loopback,
    /// Capture an input device (line-in from an interface).
    LineIn,
}

/// `[rotate]` — the scene director's auto-rotate policy (Plan 0009 Phase 3).
/// Dwell bounds are whole seconds (integers in the config, per the data shape),
/// converted to the director's internal float clock at construction.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Rotate {
    /// Auto-rotate on the dwell timer when true; manual-only (`Space`) when off.
    /// Defaults to `false` (ADR-0027): a fresh install holds one scene until the
    /// operator opts into rotation via the `toggle_auto` hotkey or `auto = true`.
    pub auto: bool,
    /// Never rotate sooner than this many seconds after the last change.
    pub min_dwell_secs: u32,
    /// Always rotate by this many seconds even through a steady passage.
    pub max_dwell_secs: u32,
    /// Let the experimental track-change novelty signal nudge rotation (wired in
    /// Phase 4). On by default but clearly experimental.
    pub track_change: bool,
}

impl Default for Rotate {
    fn default() -> Self {
        Self {
            auto: false,
            min_dwell_secs: 8,
            max_dwell_secs: 40,
            track_change: true,
        }
    }
}

/// `[output]` — which display, and whether to open borderless-fullscreen on it.
/// The derived defaults (`display = 0`, no name, `fullscreen = false`) are the
/// windowed first-run fallback.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Output {
    /// Target monitor index — the fallback when no `display_name` matches.
    pub display: usize,
    /// Preferred monitor identity, matched by name *before* the raw index:
    /// winit's monitor ordering can shift across boot/hotplug, so a stored
    /// index alone may point at the wrong screen (plan Risks). Empty/unset means
    /// "use the index".
    pub display_name: Option<String>,
    /// Open borderless-fullscreen on the target display when true; windowed
    /// otherwise. Default false, so a first run with no config is windowed.
    pub fullscreen: bool,
}

impl Config {
    /// Load config from `path`, degrading to the default on any problem: a
    /// missing file is the normal first-run case (silent); a malformed file is
    /// noted to stderr but still yields the windowed default rather than a
    /// crash (NFR section 10).
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => match toml::from_str(&text) {
                Ok(config) => config,
                Err(err) => {
                    eprintln!("config {}: {err}; using defaults", path.display());
                    Config::default()
                }
            },
            // Missing file: first run. Any other read error also degrades quietly
            // to defaults — a config we can't read must never block the show.
            Err(_) => Config::default(),
        }
    }

    /// Write the config back to `path` (best-effort), creating the parent
    /// directory if needed. A serialize or write failure is logged and
    /// otherwise ignored — a persistence miss must not crash a live show.
    pub fn save(&self, path: &Path) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match toml::to_string_pretty(self) {
            Ok(text) => {
                if let Err(err) = std::fs::write(path, text) {
                    eprintln!("could not write config {}: {err}", path.display());
                }
            }
            Err(err) => eprintln!("could not serialize config: {err}"),
        }
    }
}
