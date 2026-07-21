//! The preset layer — ADR-0002 layers 1-2: TOML data binding built-in system
//! parameters to a pure expression language over the audio analysis.
//!
//! This module is the evaluator and data model only (Plan 0003 Phase 4).
//! [`expr`] compiles and evaluates expression strings; [`schema`] parses a
//! TOML preset into compiled [`Binding`]s. Wiring these to the fragment-field
//! and swarm systems (named-parameter surfaces, hot-reload) is Phase 5.

pub mod expr;
pub mod schema;

pub use expr::{Expr, ExprError, Variables, compile};
pub use schema::{Binding, Preset, PresetError, SystemKind};
