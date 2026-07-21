//! Deterministic analysis of the PCM stream: windowed FFT spectrum plus an
//! onset envelope and beat flag, delivered once per hop as an
//! [`AnalysisFrame`].
//!
//! Everything here is a pure function of the samples fed in — no wall clock,
//! no unseeded randomness (NFR section 6). Window and hop sizes fit the 60 ms
//! latency budget at 48 kHz: one hop is ~10.7 ms (NFR section 3).

// Hot-path panic-denial pragma (Plan 0002 Phase 2). Analysis runs every hop
// off the render loop; it must never panic on valid input.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

pub mod bands;
pub mod fft;
pub mod onset;
pub mod tempo;

use crate::audio::{AudioFormat, FormatError};

/// FFT window length in samples (~43 ms at 48 kHz).
pub const WINDOW_SIZE: usize = 2048;
/// Samples between successive analysis hops (~10.7 ms at 48 kHz).
pub const HOP_SIZE: usize = 512;
/// Log-frequency bands exposed to scenes.
pub const SPECTRUM_BINS: usize = 64;

/// One hop's worth of analysis. `spectrum` values are normalized so a
/// full-scale sine lands near 1.0 in its band; `onset` is the spectral-flux
/// envelope; `beat` flags an onset event this hop.
#[derive(Debug, Clone, Copy)]
pub struct AnalysisFrame {
    /// Per-band energy, normalized so a full-scale sine reads near 1.0.
    pub spectrum: [f32; SPECTRUM_BINS],
    /// Spectral-flux onset envelope for this hop.
    pub onset: f32,
    /// Whether a beat (onset event) fired this hop.
    pub beat: bool,
    /// Mean magnitude in the bass band (~20-250 Hz).
    pub bass: f32,
    /// Mean magnitude in the mid band (~250-4000 Hz).
    pub mid: f32,
    /// Mean magnitude in the treble band (~4-18 kHz).
    pub treb: f32,
    /// Tempo estimate in BPM (hop-clock autocorrelation; 0 until warm).
    pub bpm: f32,
    /// Beat phase in [0, 1): 0 on each beat, ramping to the next.
    pub bar: f32,
}

impl Default for AnalysisFrame {
    fn default() -> Self {
        Self {
            spectrum: [0.0; SPECTRUM_BINS],
            onset: 0.0,
            beat: false,
            bass: 0.0,
            mid: 0.0,
            treb: 0.0,
            bpm: 0.0,
            bar: 0.0,
        }
    }
}

/// Stateful per-stream analyzer: accumulates interleaved samples into mono
/// hops, runs FFT + onset detection each completed hop, and hands the latest
/// frame to the render side. Deterministic for a given sample sequence.
///
/// After construction, processing allocates nothing — safe to drive from the
/// render loop every frame.
pub struct Analyzer {
    format: AudioFormat,
    spectrum: fft::SpectrumAnalyzer,
    onset: onset::OnsetDetector,
    bands: bands::BandSplitter,
    tempo: tempo::TempoTracker,
    window: [f32; WINDOW_SIZE],
    /// Valid samples in `window`; analysis starts once fully warm.
    window_filled: usize,
    hop: [f32; HOP_SIZE],
    hop_filled: usize,
    latest: AnalysisFrame,
    /// Beats are sticky between `take_frame` calls so a beat can never fall
    /// between two render frames and vanish.
    pending_beat: bool,
}

impl Analyzer {
    /// Build an analyzer for a validated stream format.
    pub fn new(format: AudioFormat) -> Result<Self, FormatError> {
        let format = format.validate()?;
        Ok(Self {
            format,
            spectrum: fft::SpectrumAnalyzer::new(format.sample_rate),
            onset: onset::OnsetDetector::new(),
            bands: bands::BandSplitter::new(format.sample_rate),
            tempo: tempo::TempoTracker::new(format.sample_rate),
            window: [0.0; WINDOW_SIZE],
            window_filled: 0,
            hop: [0.0; HOP_SIZE],
            hop_filled: 0,
            latest: AnalysisFrame::default(),
            pending_beat: false,
        })
    }

    /// The validated format this analyzer was created with.
    pub fn format(&self) -> AudioFormat {
        self.format
    }

    /// The log-frequency band a given frequency falls into — lets scenes and
    /// tests reason about where energy should show up.
    pub fn band_for_freq(&self, hz: f32) -> usize {
        self.spectrum.band_for_freq(hz)
    }

    /// Feed interleaved samples (whole frames, as produced by the intake).
    /// Runs one analysis pass per completed hop.
    #[allow(
        clippy::indexing_slicing,
        reason = "hop_filled < HOP_SIZE (reset at the boundary) and the window slice is a fixed WINDOW_SIZE-HOP_SIZE range; both are in-bounds by construction"
    )]
    pub fn push_interleaved(&mut self, samples: &[f32]) {
        let channels = self.format.channels as usize;
        for frame in samples.chunks_exact(channels) {
            let mono = frame.iter().sum::<f32>() / channels as f32;
            self.hop[self.hop_filled] = mono;
            self.hop_filled += 1;
            if self.hop_filled == HOP_SIZE {
                self.hop_filled = 0;
                self.window.copy_within(HOP_SIZE.., 0);
                self.window[WINDOW_SIZE - HOP_SIZE..].copy_from_slice(&self.hop);
                self.window_filled = (self.window_filled + HOP_SIZE).min(WINDOW_SIZE);
                if self.window_filled == WINDOW_SIZE {
                    let spectrum = self.spectrum.analyze(&self.window);
                    let (onset, beat) = self.onset.process(self.spectrum.magnitudes());
                    let (bass, mid, treb) = self.bands.split(self.spectrum.magnitudes());
                    let (bpm, bar) = self.tempo.process(onset, beat);
                    self.latest = AnalysisFrame {
                        spectrum,
                        onset,
                        beat,
                        bass,
                        mid,
                        treb,
                        bpm,
                        bar,
                    };
                    self.pending_beat |= beat;
                }
            }
        }
    }

    /// Latest analysis with any beat since the previous take. Call once per
    /// render frame.
    pub fn take_frame(&mut self) -> AnalysisFrame {
        let mut frame = self.latest;
        frame.beat = self.pending_beat;
        self.pending_beat = false;
        self.latest.beat = false;
        frame
    }
}
