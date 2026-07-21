//! Onset envelope (spectral flux) and beat flagging with an adaptive
//! threshold. Deterministic: state depends only on the magnitude sequence.

use super::WINDOW_SIZE;

const MAG_BINS: usize = WINDOW_SIZE / 2;
/// ~0.46 s of flux history at a 10.7 ms hop — enough context for an adaptive
/// threshold without smearing across musical phrases.
const HISTORY: usize = 43;
/// Minimum gap between beats: ~96 ms, i.e. faster than any plausible beat.
const REFRACTORY_HOPS: u32 = 9;
/// Beat when flux exceeds mean + K * std of recent history...
const THRESHOLD_K: f32 = 1.5;
/// ...and is at least this absolute level, so numeric dust in silence never
/// registers.
const ABS_FLOOR: f32 = 1e-6;

pub struct OnsetDetector {
    prev: [f32; MAG_BINS],
    have_prev: bool,
    history: [f32; HISTORY],
    hist_pos: usize,
    hist_len: usize,
    refractory: u32,
}

impl OnsetDetector {
    pub fn new() -> Self {
        Self {
            prev: [0.0; MAG_BINS],
            have_prev: false,
            history: [0.0; HISTORY],
            hist_pos: 0,
            hist_len: 0,
            refractory: 0,
        }
    }

    /// One hop: returns (onset envelope value, beat flag).
    pub fn process(&mut self, mags: &[f32; MAG_BINS]) -> (f32, bool) {
        // Spectral flux: mean positive magnitude increase per bin.
        let mut flux = 0.0f32;
        if self.have_prev {
            for (m, p) in mags.iter().zip(self.prev.iter()) {
                flux += (m - p).max(0.0);
            }
            flux /= MAG_BINS as f32;
        }
        self.prev.copy_from_slice(mags);
        self.have_prev = true;

        // Threshold from history *before* this hop is added, so a spike
        // cannot raise the bar against itself.
        let (mean, std) = self.history_stats();
        let over_threshold = flux > mean + THRESHOLD_K * std && flux > ABS_FLOOR;
        let beat = self.refractory == 0 && over_threshold;
        if beat {
            self.refractory = REFRACTORY_HOPS;
        } else {
            self.refractory = self.refractory.saturating_sub(1);
        }

        self.history[self.hist_pos] = flux;
        self.hist_pos = (self.hist_pos + 1) % HISTORY;
        self.hist_len = (self.hist_len + 1).min(HISTORY);

        (flux, beat)
    }

    fn history_stats(&self) -> (f32, f32) {
        if self.hist_len == 0 {
            return (0.0, 0.0);
        }
        let n = self.hist_len as f32;
        let slice = &self.history[..self.hist_len];
        let mean = slice.iter().sum::<f32>() / n;
        let var = slice.iter().map(|f| (f - mean) * (f - mean)).sum::<f32>() / n;
        (mean, var.sqrt())
    }
}

impl Default for OnsetDetector {
    fn default() -> Self {
        Self::new()
    }
}
