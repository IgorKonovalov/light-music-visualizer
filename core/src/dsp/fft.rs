//! Windowed FFT producing linear magnitudes plus a log-frequency band
//! spectrum for scenes.

use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use super::{SPECTRUM_BINS, WINDOW_SIZE};

const MAG_BINS: usize = WINDOW_SIZE / 2;
/// Log band range. The top is clamped below Nyquist for low sample rates.
const BAND_LO_HZ: f32 = 35.0;
const BAND_HI_HZ: f32 = 18_000.0;

pub struct SpectrumAnalyzer {
    fft: Arc<dyn Fft<f32>>,
    hann: [f32; WINDOW_SIZE],
    buf: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    mags: [f32; MAG_BINS],
    /// `band_edges[k]..band_edges[k+1]` are the linear bins of log band `k`.
    band_edges: [usize; SPECTRUM_BINS + 1],
    sample_rate: f32,
    /// Scales a Hann-windowed peak magnitude back to sine amplitude
    /// (Hann coherent gain 1/2, one-sided spectrum 2/N => 4/N).
    norm: f32,
}

impl SpectrumAnalyzer {
    pub fn new(sample_rate: u32) -> Self {
        let fft = FftPlanner::new().plan_fft_forward(WINDOW_SIZE);
        let scratch_len = fft.get_inplace_scratch_len();

        let mut hann = [0.0f32; WINDOW_SIZE];
        for (i, w) in hann.iter_mut().enumerate() {
            let phase = i as f32 / (WINDOW_SIZE - 1) as f32;
            *w = 0.5 - 0.5 * (std::f32::consts::TAU * phase).cos();
        }

        let sr = sample_rate as f32;
        let hi = BAND_HI_HZ.min(sr * 0.45);
        let ratio = hi / BAND_LO_HZ;
        let bin_hz = sr / WINDOW_SIZE as f32;
        let mut band_edges = [0usize; SPECTRUM_BINS + 1];
        for (k, edge) in band_edges.iter_mut().enumerate() {
            let f = BAND_LO_HZ * ratio.powf(k as f32 / SPECTRUM_BINS as f32);
            *edge = ((f / bin_hz).round() as usize).clamp(1, MAG_BINS);
        }
        // Guarantee every band at least one bin (low bands collapse when the
        // log curve is flatter than one linear bin).
        for k in 1..band_edges.len() {
            if band_edges[k] <= band_edges[k - 1] {
                band_edges[k] = (band_edges[k - 1] + 1).min(MAG_BINS);
            }
        }

        Self {
            fft,
            hann,
            buf: vec![Complex::new(0.0, 0.0); WINDOW_SIZE],
            scratch: vec![Complex::new(0.0, 0.0); scratch_len],
            mags: [0.0; MAG_BINS],
            band_edges,
            sample_rate: sr,
            norm: 4.0 / WINDOW_SIZE as f32,
        }
    }

    /// FFT the window and return the log-frequency band spectrum. Band value
    /// is the peak bin in the band, so a pure tone reads near its amplitude
    /// regardless of band width.
    pub fn analyze(&mut self, window: &[f32; WINDOW_SIZE]) -> [f32; SPECTRUM_BINS] {
        for (i, (s, w)) in window.iter().zip(self.hann.iter()).enumerate() {
            self.buf[i] = Complex::new(s * w, 0.0);
        }
        self.fft
            .process_with_scratch(&mut self.buf, &mut self.scratch);
        for (i, m) in self.mags.iter_mut().enumerate() {
            *m = self.buf[i].norm() * self.norm;
        }

        let mut bands = [0.0f32; SPECTRUM_BINS];
        for (k, band) in bands.iter_mut().enumerate() {
            let (lo, hi) = (self.band_edges[k], self.band_edges[k + 1]);
            *band = self.mags[lo..hi].iter().fold(0.0f32, |a, &b| a.max(b));
        }
        bands
    }

    /// Normalized linear magnitudes of the most recent `analyze` call
    /// (consumed by onset detection).
    pub fn magnitudes(&self) -> &[f32; MAG_BINS] {
        &self.mags
    }

    pub fn band_for_freq(&self, hz: f32) -> usize {
        let bin = (hz / (self.sample_rate / WINDOW_SIZE as f32)).round() as usize;
        let bin = bin.clamp(1, MAG_BINS);
        // Last band whose start is at or below the bin.
        (0..SPECTRUM_BINS)
            .rev()
            .find(|&k| self.band_edges[k] <= bin)
            .unwrap_or(0)
    }
}
