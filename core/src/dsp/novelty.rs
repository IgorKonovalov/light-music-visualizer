//! Long-window spectral novelty for experimental track-change detection
//! (Plan 0009 Phase 4). Feeds the standalone's scene director a *nudge*: a large
//! spectral shift (a new track / section with a different frequency
//! distribution) raises the novelty score, and the director lets that pull scene
//! rotation earlier — but never triggers a change on novelty alone, since
//! beatmatched DJ blends have no hard edge (NFR section 10).
//!
//! The score is the distance between the current per-band spectrum and a slow
//! exponential running mean of it, **normalized by the mean's magnitude** so it
//! measures a change in spectral *shape*, not loudness — a pure volume swell
//! reads ~0, a full spectral swap reads ~sqrt(2). Within a steady segment the
//! spectrum sits on its own running mean, so novelty stays near zero; at a
//! boundary the spectrum diverges from a mean still built on the previous
//! segment, so novelty spikes, then decays as the mean catches up. Deliberately
//! spectral, not tempo: a beatmatched set holds tempo across the blend, so a
//! tempo term would miss exactly the case this must stay soft on.
//!
//! Pure and deterministic — a function of the spectrum sequence alone, no wall
//! clock and no randomness (NFR section 6).

// Hot-path panic-denial pragma (Plan 0002 Phase 2): runs every hop off the
// render loop, so it must never panic on valid input.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

use super::{HOP_SIZE, SPECTRUM_BINS};

/// Time constant (seconds) of the running-mean window. ~2 s is long enough that
/// per-beat spectral wobble sits on the mean (low novelty) while a whole-track
/// change stands out against it.
const NOVELTY_TAU: f32 = 2.0;

/// Floor added to the mean magnitude in the denominator, so near-silence (a
/// tiny mean) can't blow the ratio up to a spurious spike.
const MEAN_EPS: f32 = 1e-3;

/// Tracks the slow running mean of the spectrum and reports how far the current
/// spectrum sits from it.
pub struct NoveltyDetector {
    /// Exponential running mean of the per-band spectrum.
    mean: [f32; SPECTRUM_BINS],
    /// Per-hop EMA coefficient, derived from the hop duration and `NOVELTY_TAU`.
    alpha: f32,
    /// Seeded on the first hop so the mean starts on real data, not zeros.
    warm: bool,
}

impl NoveltyDetector {
    /// Build a detector for a given sample rate (sets the per-hop smoothing so
    /// the effective window is `NOVELTY_TAU` seconds regardless of rate).
    pub fn new(sample_rate: u32) -> Self {
        let hop_dt = HOP_SIZE as f32 / sample_rate.max(1) as f32;
        let alpha = 1.0 - (-hop_dt / NOVELTY_TAU).exp();
        Self {
            mean: [0.0; SPECTRUM_BINS],
            alpha,
            warm: false,
        }
    }

    /// Consume one hop's spectrum and return the novelty score (0 on the first
    /// hop, while the mean seeds). The running mean folds in the current
    /// spectrum *after* the measurement, so a boundary spikes before the mean
    /// absorbs it.
    pub fn process(&mut self, spectrum: &[f32; SPECTRUM_BINS]) -> f32 {
        if !self.warm {
            self.mean = *spectrum;
            self.warm = true;
            return 0.0;
        }
        let dist_sq: f32 = spectrum
            .iter()
            .zip(self.mean.iter())
            .map(|(s, m)| {
                let d = s - m;
                d * d
            })
            .sum();
        let mean_energy: f32 = self.mean.iter().map(|m| m * m).sum();
        // Normalize by the mean's magnitude: a shape change, not a level change.
        let novelty = dist_sq.sqrt() / (mean_energy.sqrt() + MEAN_EPS);

        for (m, s) in self.mean.iter_mut().zip(spectrum.iter()) {
            *m += (*s - *m) * self.alpha;
        }
        novelty
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat-ish spectrum with all energy in one contiguous band range, so two
    /// different ranges are clearly distinct.
    fn spectrum(active: std::ops::Range<usize>) -> [f32; SPECTRUM_BINS] {
        let mut s = [0.0f32; SPECTRUM_BINS];
        for (i, v) in s.iter_mut().enumerate() {
            if active.contains(&i) {
                *v = 1.0;
            }
        }
        s
    }

    #[test]
    fn steady_spectrum_stays_low_and_a_change_spikes() {
        let mut d = NoveltyDetector::new(48_000);
        let low = spectrum(0..8);
        let high = spectrum(48..56);

        // Warm on a steady low-band segment: novelty settles near zero.
        for _ in 0..200 {
            d.process(&low);
        }
        let steady = d.process(&low);
        assert!(steady < 0.05, "steady novelty {steady} should be ~0");

        // Switch to a distinct high-band segment: the score spikes toward the
        // ~sqrt(2) a full spectral swap produces.
        let spike = d.process(&high);
        assert!(
            spike > 0.8,
            "a spectral change should spike novelty (got {spike}, steady {steady})"
        );
        assert!(spike > steady * 10.0, "spike {spike} vs steady {steady}");

        // Holding on the new segment lets the mean catch up, novelty decays.
        for _ in 0..400 {
            d.process(&high);
        }
        let settled = d.process(&high);
        assert!(
            settled < spike * 0.5,
            "novelty should decay within the new segment (settled {settled}, spike {spike})"
        );
    }

    #[test]
    fn first_hop_has_no_novelty() {
        let mut d = NoveltyDetector::new(48_000);
        assert_eq!(d.process(&spectrum(0..8)), 0.0);
    }
}
