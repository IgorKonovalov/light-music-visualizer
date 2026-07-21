//! Source-agnostic sample intake: a lock-free SPSC ring buffer.
//!
//! The producer side lives on an audio thread (WASAPI capture, foobar's
//! `visualisation_stream`, ...) and must stay real-time safe: `push_samples`
//! performs no allocation, no locks, no logging, no I/O (NFR section 5).
//! Format (sample rate, channel count) is validated once when the intake is
//! created — the boundary — and the hot path trusts it from then on.
//!
//! Samples are interleaved f32 frames. If the ring is full the producer drops
//! the excess (never blocks); the consumer is expected to drain every render
//! frame so reads stay near the write head (NFR section 3).

// Hot-path panic-denial pragma (Plan 0002 Phase 2). The audio callback and
// ring must never panic in production; violations fail the build.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

use std::cell::UnsafeCell;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

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
    let capacity_samples = (capacity_frames.max(1) * format.channels as usize).next_power_of_two();
    let shared = Arc::new(RingShared {
        buf: (0..capacity_samples)
            .map(|_| UnsafeCell::new(0.0))
            .collect(),
        mask: capacity_samples - 1,
        head: PaddedAtomicUsize::new(0),
        tail: PaddedAtomicUsize::new(0),
    });
    Ok((
        SampleProducer {
            shared: Arc::clone(&shared),
            format,
        },
        SampleConsumer { shared, format },
    ))
}

/// Keep head and tail on separate cache lines so the two threads do not
/// false-share.
#[repr(align(64))]
struct PaddedAtomicUsize(AtomicUsize);

impl PaddedAtomicUsize {
    fn new(v: usize) -> Self {
        Self(AtomicUsize::new(v))
    }
}

struct RingShared {
    buf: Box<[UnsafeCell<f32>]>,
    mask: usize,
    /// Total samples ever written (monotonic); producer-owned.
    head: PaddedAtomicUsize,
    /// Total samples ever read (monotonic); consumer-owned.
    tail: PaddedAtomicUsize,
}

// Safety: head/tail are atomics; each buffer slot is written only by the
// single producer while unpublished (before the head store) and read only by
// the single consumer after the Release/Acquire handoff publishes it.
unsafe impl Send for RingShared {}
unsafe impl Sync for RingShared {}

/// Audio-thread half. Real-time safe: `push_samples` never allocates,
/// locks, or blocks.
pub struct SampleProducer {
    shared: Arc<RingShared>,
    format: AudioFormat,
}

impl SampleProducer {
    /// The validated format this producer was created with.
    pub fn format(&self) -> AudioFormat {
        self.format
    }

    /// Push interleaved samples; `samples.len()` must be a multiple of the
    /// channel count (whole frames — the capture API's contract, checked in
    /// debug builds only to keep the hot path free).
    /// Returns how many samples were written; the rest are dropped if the
    /// ring is full (dropping is the real-time-safe overflow policy).
    #[allow(
        clippy::indexing_slicing,
        reason = "ring indices are masked (& mask) and `samples[..n]` is bounded by n = min(len, free); see the Safety notes"
    )]
    pub fn push_samples(&mut self, samples: &[f32]) -> usize {
        debug_assert_eq!(samples.len() % self.format.channels as usize, 0);
        let head = self.shared.head.0.load(Ordering::Relaxed);
        let tail = self.shared.tail.0.load(Ordering::Acquire);
        let free = self.shared.buf.len() - (head - tail);
        // Round down to whole frames so a partial push never splits a frame
        // and desynchronizes channel interleaving for the consumer.
        let n =
            samples.len().min(free) / self.format.channels as usize * self.format.channels as usize;
        for (i, &s) in samples[..n].iter().enumerate() {
            let idx = (head + i) & self.shared.mask;
            // Safety: slots in [head, head + free) are unpublished — only the
            // producer touches them until the Release store below.
            unsafe { *self.shared.buf[idx].get() = s };
        }
        self.shared.head.0.store(head + n, Ordering::Release);
        n
    }
}

/// Render/DSP-thread half.
pub struct SampleConsumer {
    shared: Arc<RingShared>,
    format: AudioFormat,
}

impl SampleConsumer {
    /// The validated format this consumer was created with.
    pub fn format(&self) -> AudioFormat {
        self.format
    }

    /// Interleaved samples currently readable.
    pub fn available(&self) -> usize {
        let head = self.shared.head.0.load(Ordering::Acquire);
        let tail = self.shared.tail.0.load(Ordering::Relaxed);
        head - tail
    }

    /// Pop up to `out.len()` interleaved samples; returns how many were read.
    #[allow(
        clippy::indexing_slicing,
        reason = "ring indices are masked (& mask) and `out[..n]` is bounded by n = min(len, available); see the Safety notes"
    )]
    pub fn pop_samples(&mut self, out: &mut [f32]) -> usize {
        let head = self.shared.head.0.load(Ordering::Acquire);
        let tail = self.shared.tail.0.load(Ordering::Relaxed);
        let n = out.len().min(head - tail);
        for (i, slot) in out[..n].iter_mut().enumerate() {
            let idx = (tail + i) & self.shared.mask;
            // Safety: slots in [tail, head) were published by the producer's
            // Release store and are not rewritten until we advance tail.
            *slot = unsafe { *self.shared.buf[idx].get() };
        }
        self.shared.tail.0.store(tail + n, Ordering::Release);
        n
    }
}

#[cfg(test)]
mod tests {
    // Tests index fixed-size arrays and known-length buffers freely; the
    // hot-path pragma above cascades here, so re-allow indexing for tests.
    #![allow(clippy::indexing_slicing)]

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

    #[test]
    fn roundtrip_preserves_order() {
        let (mut tx, mut rx) = intake(fmt(48_000, 1), 8).unwrap();
        assert_eq!(tx.push_samples(&[1.0, 2.0, 3.0]), 3);
        let mut out = [0.0; 3];
        assert_eq!(rx.pop_samples(&mut out), 3);
        assert_eq!(out, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn wraparound_keeps_data_intact() {
        let (mut tx, mut rx) = intake(fmt(48_000, 1), 4).unwrap();
        let mut out = [0.0; 4];
        for round in 0..10 {
            let base = round as f32 * 4.0;
            assert_eq!(
                tx.push_samples(&[base, base + 1.0, base + 2.0, base + 3.0]),
                4
            );
            assert_eq!(rx.pop_samples(&mut out), 4);
            assert_eq!(out, [base, base + 1.0, base + 2.0, base + 3.0]);
        }
    }

    #[test]
    fn full_ring_drops_excess_instead_of_blocking() {
        let (mut tx, mut rx) = intake(fmt(48_000, 1), 4).unwrap();
        assert_eq!(tx.push_samples(&[1.0, 2.0, 3.0, 4.0]), 4);
        assert_eq!(tx.push_samples(&[5.0]), 0);
        let mut out = [0.0; 4];
        assert_eq!(rx.pop_samples(&mut out), 4);
        assert_eq!(out, [1.0, 2.0, 3.0, 4.0]);
        assert_eq!(tx.push_samples(&[5.0]), 1);
        assert_eq!(rx.pop_samples(&mut out[..1]), 1);
        assert_eq!(out[0], 5.0);
    }

    #[test]
    fn cross_thread_stream_arrives_in_order() {
        let (mut tx, mut rx) = intake(fmt(48_000, 2), 1024).unwrap();
        let total: usize = 100_000;
        let writer = std::thread::spawn(move || {
            let mut sent = 0usize;
            let mut chunk = [0.0f32; 64];
            while sent < total {
                let n = chunk.len().min(total - sent);
                for (i, s) in chunk[..n].iter_mut().enumerate() {
                    *s = (sent + i) as f32;
                }
                sent += tx.push_samples(&chunk[..n]);
            }
        });
        let mut expected = 0usize;
        let mut buf = [0.0f32; 256];
        while expected < total {
            let n = rx.pop_samples(&mut buf);
            for &s in &buf[..n] {
                assert_eq!(s, expected as f32);
                expected += 1;
            }
        }
        writer.join().unwrap();
    }
}
