//! lmv-core — the shared, source-agnostic brain of light-music-visualizer.
//!
//! Takes PCM frames in (it never knows whether they came from loopback capture
//! or foobar2000), runs DSP (spectrum, onset/beat), and renders scenes via wgpu.
//! See ADR-0001 for the architecture and the layering rules in CLAUDE.md:
//! no audio-source or platform types in this crate, ever.

pub mod audio;
