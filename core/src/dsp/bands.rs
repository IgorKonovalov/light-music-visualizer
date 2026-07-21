//! Band-energy split: fixed Hz cutoffs over the linear FFT magnitudes into
//! three mean energies — bass, mid, treble — for scenes and presets.
//!
//! Pure and allocation-free after construction: the linear-bin range of each
//! band is precomputed from the sample rate, and each split is an iterator
//! mean over those bins, so the same magnitudes always yield the same triple
//! (determinism, NFR 6).

// Hot-path panic-denial pragma (Plan 0002 Phase 2). Runs every analysis hop.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

use super::WINDOW_SIZE;

/// Linear magnitude bins available (one-sided spectrum).
const MAG_BINS: usize = WINDOW_SIZE / 2;
/// Band cutoffs in Hz. Bass starts above DC/rumble; treble tops out below the
/// air band and is clamped to Nyquist on low sample rates.
const BASS_LO_HZ: f32 = 20.0;
const BASS_HI_HZ: f32 = 250.0;
const MID_HI_HZ: f32 = 4_000.0;
const TREB_HI_HZ: f32 = 18_000.0;

/// Precomputed `(lo, hi)` linear-bin ranges (half-open) for the three bands.
pub struct BandSplitter {
    bass: (usize, usize),
    mid: (usize, usize),
    treb: (usize, usize),
}

impl BandSplitter {
    /// Precompute the per-band bin ranges for `sample_rate`.
    pub fn new(sample_rate: u32) -> Self {
        let bin_hz = sample_rate as f32 / WINDOW_SIZE as f32;
        // Bin nearest a frequency, kept in [1, MAG_BINS] (bin 0 is DC).
        let bin = |hz: f32| ((hz / bin_hz).round() as usize).clamp(1, MAG_BINS);
        let bass_lo = bin(BASS_LO_HZ);
        let bass_hi = bin(BASS_HI_HZ);
        let mid_hi = bin(MID_HI_HZ);
        let treb_hi = bin(TREB_HI_HZ);
        Self {
            bass: (bass_lo, bass_hi),
            mid: (bass_hi, mid_hi),
            treb: (mid_hi, treb_hi),
        }
    }

    /// `(bass, mid, treb)` mean magnitude over each band's linear bins.
    pub fn split(&self, mags: &[f32]) -> (f32, f32, f32) {
        (
            band_mean(mags, self.bass),
            band_mean(mags, self.mid),
            band_mean(mags, self.treb),
        )
    }
}

/// Mean of `mags` over the half-open bin range `[lo, hi)`, via iterators so no
/// indexing pragma escape is needed. Empty range reads 0.
fn band_mean(mags: &[f32], (lo, hi): (usize, usize)) -> f32 {
    let n = hi.saturating_sub(lo);
    if n == 0 {
        return 0.0;
    }
    let sum: f32 = mags.iter().skip(lo).take(n).sum();
    sum / n as f32
}
