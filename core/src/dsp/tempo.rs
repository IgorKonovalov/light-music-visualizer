//! Deterministic tempo (BPM) and beat-phase (`bar`) from the onset envelope.
//!
//! The time base is the analyzer's hop count, never the wall clock: BPM is the
//! lag of the strongest mean-subtracted autocorrelation of the recent onset
//! envelope (parabolically refined for sub-hop precision), and `bar` is a 0..1
//! phase advanced each hop by the current BPM and snapped to 0 on every
//! detected beat. Pure and allocation-free after construction — the envelope
//! history is a fixed array and every pass is iterator-based, so the same
//! `(onset, beat)` sequence always yields the same `(bpm, bar)` sequence
//! (NFR 6, hot-path discipline §5).

// Hot-path panic-denial pragma (Plan 0002 Phase 2). Runs every analysis hop.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

use super::HOP_SIZE;

/// Onset-envelope history (~4.1 s at a 10.7 ms hop): long enough to resolve
/// tempos down to `MIN_BPM` with several beat periods of overlap.
const ENV_HISTORY: usize = 384;
/// Tempo search range. Sub/‑super-harmonics outside this are ignored.
const MIN_BPM: f32 = 60.0;
const MAX_BPM: f32 = 200.0;

/// Rolling onset-envelope autocorrelator producing a BPM estimate and a beat
/// phase.
pub struct TempoTracker {
    /// Seconds per hop — the fixed conversion between lag (hops) and BPM.
    hop_sec: f32,
    /// Envelope history, oldest at index 0, newest at the end.
    env: [f32; ENV_HISTORY],
    /// Hops seen so far, saturating at `ENV_HISTORY` (estimation waits until
    /// the buffer is full so the autocorrelation has full context).
    filled: usize,
    /// Lag search bounds (hops) derived from `MAX_BPM`/`MIN_BPM`.
    min_lag: usize,
    max_lag: usize,
    /// Latest BPM estimate (0 until warm / when no periodicity is found).
    bpm: f32,
    /// Beat phase in [0, 1): 0 at each beat, ramping toward the next.
    phase: f32,
}

impl TempoTracker {
    /// Build a tracker for `sample_rate`, precomputing the lag search bounds.
    pub fn new(sample_rate: u32) -> Self {
        let hop_sec = HOP_SIZE as f32 / sample_rate as f32;
        // lag_hops = 60 / (bpm * hop_sec); faster tempo => shorter lag.
        let min_lag = (60.0 / (MAX_BPM * hop_sec)).floor() as usize;
        let max_lag = ((60.0 / (MIN_BPM * hop_sec)).ceil() as usize).min(ENV_HISTORY - 1);
        Self {
            hop_sec,
            env: [0.0; ENV_HISTORY],
            filled: 0,
            min_lag: min_lag.max(1),
            max_lag,
            bpm: 0.0,
            phase: 0.0,
        }
    }

    /// Advance one hop. Returns `(bpm, bar)` — `bar` is the 0..1 beat phase.
    pub fn process(&mut self, onset: f32, beat: bool) -> (f32, f32) {
        // Slide the newest onset into the tail (oldest falls off the front).
        self.env.copy_within(1.., 0);
        if let Some(last) = self.env.last_mut() {
            *last = onset;
        }
        self.filled = (self.filled + 1).min(ENV_HISTORY);

        if self.filled >= ENV_HISTORY {
            self.bpm = self.estimate_bpm();
        }

        // Beat phase: hard-reset on a detected beat so the ramp stays locked
        // to the music; otherwise advance by the current tempo.
        if beat {
            self.phase = 0.0;
        } else if self.bpm > 0.0 {
            self.phase += self.bpm * self.hop_sec / 60.0;
            self.phase -= self.phase.floor(); // wrap into [0, 1)
        }

        (self.bpm, self.phase)
    }

    /// Lag of the strongest mean-subtracted autocorrelation peak in the search
    /// range, refined to sub-hop precision, converted to BPM. Keeps the last
    /// estimate if no positive periodicity is present.
    fn estimate_bpm(&self) -> f32 {
        let mean = self.env.iter().sum::<f32>() / ENV_HISTORY as f32;

        let mut best_lag = self.min_lag;
        let mut best = f32::NEG_INFINITY;
        for lag in self.min_lag..=self.max_lag {
            let c = self.corr_at(lag, mean);
            if c > best {
                best = c;
                best_lag = lag;
            }
        }
        if best <= 0.0 {
            return self.bpm;
        }

        // Parabolic interpolation across the peak's neighbors for sub-hop lag
        // precision (keeps the estimate off the coarse integer-lag grid).
        let refined = if best_lag > self.min_lag && best_lag < self.max_lag {
            let yl = self.corr_at(best_lag - 1, mean);
            let yr = self.corr_at(best_lag + 1, mean);
            let denom = yl - 2.0 * best + yr;
            let delta = if denom.abs() > f32::EPSILON {
                (0.5 * (yl - yr) / denom).clamp(-0.5, 0.5)
            } else {
                0.0
            };
            best_lag as f32 + delta
        } else {
            best_lag as f32
        };

        60.0 / (refined * self.hop_sec)
    }

    /// Mean-subtracted autocorrelation at `lag`, iterator-based (no indexing).
    fn corr_at(&self, lag: usize, mean: f32) -> f32 {
        self.env
            .iter()
            .zip(self.env.iter().skip(lag))
            .map(|(x, y)| (x - mean) * (y - mean))
            .sum()
    }
}
