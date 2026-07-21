//! Built-in scenes. The `Scene` trait + registry land in Plan 0001 Phase 5;
//! per ADR-0002 they stay thin and crate-internal — the future preset
//! engine's rendering vocabulary, not a public extension point.

pub mod spectrum;
