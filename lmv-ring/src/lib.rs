//! A lock-free single-producer/single-consumer ring of interleaved `f32`
//! samples — the one piece of pure-Rust `unsafe` in the project.
//!
//! The producer side lives on an audio thread (WASAPI capture, foobar's
//! `visualisation_stream`, ...) and must stay real-time safe: `push_samples`
//! performs no allocation, no locks, no logging, no I/O (NFR section 5).
//!
//! Samples are interleaved `f32` frames. If the ring is full the producer drops
//! the excess (never blocks); the consumer is expected to drain every render
//! frame so reads stay near the write head (NFR section 3).
//!
//! This crate is deliberately dependency-free (no wgpu, no audio-source, no
//! platform types) so `cargo +nightly miri test -p lmv-ring` compiles and runs
//! the SPSC unit tests under Miri in seconds — the fast UB gate the extraction
//! exists for (Plan 0005). Format validation and the audio-facing `intake`
//! wrapper live in `lmv-core`'s `audio` module, which re-exports these types.

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

/// Create an SPSC pair over a ring sized to hold at least `capacity_samples`
/// interleaved samples (rounded up to a power of two so the index mask is
/// valid). `channels` is the interleaving width the producer rounds pushes to
/// so a partial write never splits a frame; the caller (lmv-core's `intake`)
/// has already validated it.
pub fn spsc(capacity_samples: usize, channels: u16) -> (SampleProducer, SampleConsumer) {
    let capacity = capacity_samples.max(1).next_power_of_two();
    let shared = Arc::new(RingShared {
        buf: (0..capacity).map(|_| UnsafeCell::new(0.0)).collect(),
        mask: capacity - 1,
        head: PaddedAtomicUsize::new(0),
        tail: PaddedAtomicUsize::new(0),
    });
    (
        SampleProducer {
            shared: Arc::clone(&shared),
            channels,
        },
        SampleConsumer { shared },
    )
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
    channels: u16,
}

impl SampleProducer {
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
        debug_assert_eq!(samples.len() % self.channels as usize, 0);
        let head = self.shared.head.0.load(Ordering::Relaxed);
        let tail = self.shared.tail.0.load(Ordering::Acquire);
        let free = self.shared.buf.len() - (head - tail);
        // Round down to whole frames so a partial push never splits a frame
        // and desynchronizes channel interleaving for the consumer.
        let n = samples.len().min(free) / self.channels as usize * self.channels as usize;
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
}

impl SampleConsumer {
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

    #[test]
    fn roundtrip_preserves_order() {
        let (mut tx, mut rx) = spsc(8, 1);
        assert_eq!(tx.push_samples(&[1.0, 2.0, 3.0]), 3);
        let mut out = [0.0; 3];
        assert_eq!(rx.pop_samples(&mut out), 3);
        assert_eq!(out, [1.0, 2.0, 3.0]);
    }

    #[test]
    fn wraparound_keeps_data_intact() {
        let (mut tx, mut rx) = spsc(4, 1);
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
        let (mut tx, mut rx) = spsc(4, 1);
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
        let (mut tx, mut rx) = spsc(2048, 2);
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
