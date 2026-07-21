//! lmv-core — the shared, source-agnostic brain of light-music-visualizer.
//!
//! Takes PCM frames in (it never knows whether they came from loopback capture
//! or foobar2000), runs DSP (spectrum, onset/beat), and renders scenes via wgpu.
//! See ADR-0001 for the architecture and the layering rules in CLAUDE.md:
//! no audio-source or platform types in this crate, ever.

// core is the shared public-API crate; its surface stays documented. Binding
// under CI's `-D warnings` by design (Plan 0002 Phase 0).
#![warn(missing_docs)]

pub mod audio;
pub mod dsp;
pub mod ffi;
pub mod preset;
pub mod render;
