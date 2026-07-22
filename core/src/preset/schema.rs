//! TOML preset schema: which built-in system a preset drives and the
//! expression bound to each of its named parameters.
//!
//! Parsing happens once at load: the raw TOML is deserialized, each parameter
//! expression is compiled (a malformed one is rejected with a surfaced error),
//! and the result is an in-memory [`Preset`] whose bindings are ready to
//! evaluate. A bad preset returns `Err` — it never panics, so the caller can
//! degrade to the last good preset (ADR-0002 / NFR 10).

use std::collections::BTreeMap;
use std::fmt;

use serde::Deserialize;

use super::expr::{self, Expr, ExprError};
use crate::render::scenes::lines::{CurveFamily, GeneratorConfig, MAX_LSYSTEM_DEPTH, hankin};

/// The built-in system a preset drives. Extend as Plan 0003 (and later plans)
/// add systems; unknown names are rejected at load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SystemKind {
    /// The fullscreen fragment-field scene.
    FragmentField,
    /// The CPU particle-swarm scene.
    Swarm,
    /// The parametric line-curve scene (Maurer rose, ...) — ADR-0007.
    ParametricCurve,
    /// The L-system generator scene — ADR-0007.
    LSystem,
    /// The Hankin star-pattern generator scene — ADR-0007.
    StarPattern,
}

impl SystemKind {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "fragment_field" => SystemKind::FragmentField,
            "swarm" => SystemKind::Swarm,
            "parametric_curve" => SystemKind::ParametricCurve,
            "lsystem" => SystemKind::LSystem,
            "star_pattern" => SystemKind::StarPattern,
            _ => return None,
        })
    }
}

/// A named parameter bound to a compiled expression.
#[derive(Debug)]
pub struct Binding {
    /// The system parameter this drives (e.g. `warp`, `hue`).
    pub name: String,
    /// The compiled expression producing its per-frame value.
    pub expr: Expr,
}

/// A loaded, ready-to-evaluate preset.
#[derive(Debug)]
pub struct Preset {
    /// Human-readable name (defaults to the system name if omitted).
    pub name: String,
    /// Which built-in system this preset drives.
    pub system: SystemKind,
    /// Parameter bindings, sorted by name for deterministic iteration.
    pub params: Vec<Binding>,
    /// Declarative structural config for a line scene (ADR-0007), applied once
    /// at preset load via `Scene::configure`. `None` for the fragment/swarm
    /// systems and for curve presets that accept the family default.
    pub config: Option<GeneratorConfig>,
}

impl Preset {
    /// Parse and compile a preset from a TOML source string.
    pub fn from_toml_str(src: &str) -> Result<Self, PresetError> {
        let raw: RawPreset = toml::from_str(src).map_err(PresetError::Toml)?;
        let system = SystemKind::from_name(&raw.system)
            .ok_or_else(|| PresetError::UnknownSystem(raw.system.clone()))?;
        let name = raw.name.unwrap_or_else(|| raw.system.clone());

        // The raw params come from a BTreeMap, so bindings land name-sorted:
        // evaluation is order-independent, but determinism is cheap to keep.
        let mut params = Vec::with_capacity(raw.params.len());
        for (param, source) in raw.params {
            let expr = expr::compile(&source).map_err(|err| PresetError::Expr {
                param: param.clone(),
                err,
            })?;
            params.push(Binding { name: param, expr });
        }

        // Structural config: validated once here (a bad family/grammar -> load
        // error, the caller keeps the last good preset), then trusted by the
        // scene. Built per system so each reads the right `[curve]`/`[generator]`
        // table.
        let config = build_config(system, raw.curve, raw.generator)?;

        Ok(Preset {
            name,
            system,
            params,
            config,
        })
    }
}

/// Assemble the optional structural config for `system` from the raw tables,
/// validating at this boundary (ADR-0007). Non-line systems have no config.
fn build_config(
    system: SystemKind,
    curve: Option<RawCurve>,
    generator: Option<RawGenerator>,
) -> Result<Option<GeneratorConfig>, PresetError> {
    match system {
        // A curve preset without a `[curve]` table accepts the family default.
        SystemKind::ParametricCurve => curve.map(RawCurve::into_config).transpose(),
        // A generator preset must declare its `[generator]` table.
        SystemKind::LSystem => {
            let g = generator.ok_or_else(|| {
                PresetError::Config("lsystem requires a [generator] table".into())
            })?;
            Ok(Some(g.into_lsystem()?))
        }
        SystemKind::StarPattern => {
            let g = generator.ok_or_else(|| {
                PresetError::Config("star_pattern requires a [generator] table".into())
            })?;
            Ok(Some(g.into_star()?))
        }
        SystemKind::FragmentField | SystemKind::Swarm => Ok(None),
    }
}

/// The on-disk shape, before expressions are compiled.
#[derive(Deserialize)]
struct RawPreset {
    system: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    params: BTreeMap<String, String>,
    /// The optional `[curve]` structural-config table (ADR-0007), present on
    /// parametric-curve presets.
    #[serde(default)]
    curve: Option<RawCurve>,
    /// The optional `[generator]` structural-config table (ADR-0007), present on
    /// generator presets (L-system, star pattern).
    #[serde(default)]
    generator: Option<RawGenerator>,
}

/// The raw `[curve]` table: declarative structure for a parametric-curve scene.
#[derive(Deserialize)]
struct RawCurve {
    /// Curve family name (e.g. `"maurer_rose"`).
    family: String,
}

impl RawCurve {
    /// Validate the family name into a [`GeneratorConfig`], erroring (never
    /// panicking) on an unknown family.
    fn into_config(self) -> Result<GeneratorConfig, PresetError> {
        let family = CurveFamily::from_name(&self.family).ok_or_else(|| {
            PresetError::Config(format!("unknown curve family '{}'", self.family))
        })?;
        Ok(GeneratorConfig::Curve { family })
    }
}

/// The raw `[generator]` table: declarative structure for a generator scene.
/// Fields are optional at the serde layer and validated per system below, so
/// one table shape can serve the L-system (and, later, the star pattern).
#[derive(Deserialize)]
struct RawGenerator {
    /// L-system: starting string.
    #[serde(default)]
    axiom: Option<String>,
    /// L-system: production rules, each key a single predecessor character.
    #[serde(default)]
    rules: BTreeMap<String, String>,
    /// L-system: turn angle in degrees.
    #[serde(default)]
    angle_deg: Option<f32>,
    /// L-system: iterations to precompute.
    #[serde(default)]
    max_depth: Option<u32>,
    /// L-system: reserved seed (deterministic today).
    #[serde(default)]
    seed: Option<u64>,
    /// Star pattern: the regular tiling (e.g. `"6.6.6"` / `"hexagon"` / `"12"`).
    #[serde(default)]
    tiling: Option<String>,
    /// Star pattern: contact angle in degrees.
    #[serde(default)]
    contact_angle_deg: Option<f32>,
}

impl RawGenerator {
    /// Validate the table as an L-system config: a non-empty axiom, single-char
    /// rule predecessors, a finite angle, and a depth in `1..=MAX_LSYSTEM_DEPTH`.
    /// Every failure is a surfaced load error, never a panic (ADR-0007).
    fn into_lsystem(self) -> Result<GeneratorConfig, PresetError> {
        let axiom = self
            .axiom
            .filter(|a| !a.is_empty())
            .ok_or_else(|| PresetError::Config("lsystem needs a non-empty axiom".into()))?;

        let mut rules = Vec::with_capacity(self.rules.len());
        for (pred, succ) in self.rules {
            let mut chars = pred.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return Err(PresetError::Config(format!(
                    "lsystem rule key '{pred}' must be a single character"
                )));
            };
            rules.push((c, succ));
        }
        if rules.is_empty() {
            return Err(PresetError::Config(
                "lsystem needs at least one rule".into(),
            ));
        }

        let angle_deg = self.angle_deg.unwrap_or(25.0);
        if !angle_deg.is_finite() {
            return Err(PresetError::Config(
                "lsystem angle_deg must be finite".into(),
            ));
        }

        let max_depth = self.max_depth.unwrap_or(4);
        if max_depth == 0 || max_depth > MAX_LSYSTEM_DEPTH {
            return Err(PresetError::Config(format!(
                "lsystem max_depth must be 1..={MAX_LSYSTEM_DEPTH}, got {max_depth}"
            )));
        }

        Ok(GeneratorConfig::LSystem {
            axiom,
            rules,
            angle_deg,
            max_depth,
            seed: self.seed.unwrap_or(0),
        })
    }

    /// Validate the table as a star-pattern config: a known regular tiling and a
    /// finite contact angle. Every failure is a surfaced load error (ADR-0007).
    fn into_star(self) -> Result<GeneratorConfig, PresetError> {
        let tiling = self
            .tiling
            .ok_or_else(|| PresetError::Config("star_pattern needs a tiling".into()))?;
        let order = hankin::tiling_order(&tiling)
            .ok_or_else(|| PresetError::Config(format!("unknown tiling '{tiling}'")))?;

        let contact_angle_deg = self.contact_angle_deg.unwrap_or(30.0);
        if !contact_angle_deg.is_finite() {
            return Err(PresetError::Config(
                "star_pattern contact_angle_deg must be finite".into(),
            ));
        }

        Ok(GeneratorConfig::Star {
            order,
            contact_angle_deg,
        })
    }
}

/// Why a preset failed to load. Every variant is recoverable — the caller
/// keeps the previous good preset.
#[derive(Debug)]
pub enum PresetError {
    /// The TOML itself was malformed.
    Toml(toml::de::Error),
    /// `system` named a built-in that does not exist.
    UnknownSystem(String),
    /// A parameter's expression failed to compile.
    Expr {
        /// The parameter whose expression was invalid.
        param: String,
        /// The compile error.
        err: ExprError,
    },
    /// A structural-config table (`[curve]`/`[generator]`) was invalid — an
    /// unknown family, an out-of-range value, an undefined grammar symbol.
    Config(String),
    /// The preset file could not be read (message from the I/O error).
    Io(String),
}

impl fmt::Display for PresetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PresetError::Toml(e) => write!(f, "invalid preset TOML: {e}"),
            PresetError::UnknownSystem(s) => write!(f, "unknown system '{s}'"),
            PresetError::Expr { param, err } => {
                write!(f, "parameter '{param}' has an invalid expression: {err}")
            }
            PresetError::Config(msg) => write!(f, "invalid structural config: {msg}"),
            PresetError::Io(msg) => write!(f, "could not read preset file: {msg}"),
        }
    }
}

impl std::error::Error for PresetError {}
