//! Source-agnostic sample intake: format validation plus a lock-free SPSC ring
//! buffer.
//!
//! Format (sample rate, channel count) is validated once when the intake is
//! created — the boundary — and the hot path trusts it from then on. The ring
//! itself lives in the dependency-free [`lmv_ring`] crate (so Miri can check
//! its `unsafe` without compiling the wgpu graph, Plan 0005); its
//! [`SampleProducer`]/[`SampleConsumer`] handles are re-exported here so this
//! module stays the single audio-intake surface for the standalone and FFI.
//!
//! Samples are interleaved f32 frames. The producer side lives on an audio
//! thread and stays real-time safe (no allocation, locks, logging, or I/O in
//! `push_samples`, NFR section 5); if the ring is full the producer drops the
//! excess (never blocks), and the consumer drains every render frame (NFR
//! section 3).

// Hot-path panic-denial pragma (Plan 0002 Phase 2). The audio callback and
// ring must never panic in production; violations fail the build.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

// The SPSC ring internals moved to `lmv-ring` (Plan 0005); re-export the
// handles so the public `audio` API and every call site stay unchanged.
pub use lmv_ring::{SampleConsumer, SampleProducer};

/// Lowest sample rate the intake accepts (Hz).
pub const MIN_SAMPLE_RATE: u32 = 8_000;
/// Highest sample rate the intake accepts (Hz).
pub const MAX_SAMPLE_RATE: u32 = 384_000;
/// Most interleaved channels the intake accepts.
pub const MAX_CHANNELS: u16 = 8;

/// PCM stream format, checked once at the intake boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioFormat {
    /// Frames per second, in `MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE`.
    pub sample_rate: u32,
    /// Interleaved channel count, in `1..=MAX_CHANNELS`.
    pub channels: u16,
}

/// Why an [`AudioFormat`] was rejected at the boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatError {
    /// Sample rate fell outside `MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE`.
    SampleRateOutOfRange(u32),
    /// Channel count fell outside `1..=MAX_CHANNELS`.
    ChannelsOutOfRange(u16),
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatError::SampleRateOutOfRange(sr) => {
                write!(
                    f,
                    "sample rate {sr} outside {MIN_SAMPLE_RATE}..={MAX_SAMPLE_RATE}"
                )
            }
            FormatError::ChannelsOutOfRange(ch) => {
                write!(f, "channel count {ch} outside 1..={MAX_CHANNELS}")
            }
        }
    }
}

impl std::error::Error for FormatError {}

impl AudioFormat {
    /// Check the rate and channel bounds; the hot path trusts the result.
    pub fn validate(self) -> Result<Self, FormatError> {
        if !(MIN_SAMPLE_RATE..=MAX_SAMPLE_RATE).contains(&self.sample_rate) {
            return Err(FormatError::SampleRateOutOfRange(self.sample_rate));
        }
        if self.channels == 0 || self.channels > MAX_CHANNELS {
            return Err(FormatError::ChannelsOutOfRange(self.channels));
        }
        Ok(self)
    }
}

/// Create a validated intake: an SPSC pair sized for at least
/// `capacity_frames` frames of headroom (rounded up to a power of two).
pub fn intake(
    format: AudioFormat,
    capacity_frames: usize,
) -> Result<(SampleProducer, SampleConsumer), FormatError> {
    let format = format.validate()?;
    let capacity_samples = capacity_frames.max(1) * format.channels as usize;
    Ok(lmv_ring::spsc(capacity_samples, format.channels))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(sample_rate: u32, channels: u16) -> AudioFormat {
        AudioFormat {
            sample_rate,
            channels,
        }
    }

    #[test]
    fn format_validation_rejects_out_of_range() {
        assert!(fmt(48_000, 2).validate().is_ok());
        assert!(matches!(
            fmt(4_000, 2).validate(),
            Err(FormatError::SampleRateOutOfRange(4_000))
        ));
        assert!(matches!(
            fmt(48_000, 0).validate(),
            Err(FormatError::ChannelsOutOfRange(0))
        ));
        assert!(matches!(
            fmt(48_000, 9).validate(),
            Err(FormatError::ChannelsOutOfRange(9))
        ));
    }
}
