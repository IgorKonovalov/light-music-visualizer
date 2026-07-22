//! Pure, deterministic PCM signal synthesis for the capture / visual-QA path
//! (Plan 0013). These generate synthetic *signals* by math over a sample clock —
//! they are **not** an audio *source* (no WASAPI, no file, no OS), so they live
//! in the source-agnostic core and can be fed straight through the real
//! [`Analyzer`](crate::dsp::Analyzer) to exercise the actual DSP.
//!
//! Every generator is a pure function of its arguments — no wall clock, seeded
//! randomness only (NFR section 6). Output is interleaved f32 frames matching
//! the given [`AudioFormat`], the same shape a frontend pushes into the intake.

use std::f32::consts::TAU;

use crate::audio::AudioFormat;

/// A pure sine at `freq_hz` and `amplitude` for `secs`, interleaved to
/// `format.channels`.
pub fn sine(freq_hz: f32, secs: f32, amplitude: f32, format: AudioFormat) -> Vec<f32> {
    let sr = format.sample_rate as f32;
    let n = frame_count(secs, format.sample_rate);
    let mono: Vec<f32> = (0..n)
        .map(|i| amplitude * (TAU * freq_hz * i as f32 / sr).sin())
        .collect();
    interleave(&mono, format.channels)
}

/// A strong low-frequency sine (bass band). Thin wrapper over [`sine`].
pub fn bass_sine(freq_hz: f32, secs: f32, format: AudioFormat) -> Vec<f32> {
    sine(freq_hz, secs, 0.9, format)
}

/// A strong high-frequency sine (treble band). Thin wrapper over [`sine`].
pub fn treble_tone(freq_hz: f32, secs: f32, format: AudioFormat) -> Vec<f32> {
    sine(freq_hz, secs, 0.9, format)
}

/// Seeded white noise in `[-amplitude, amplitude]`, deterministic per `seed`.
pub fn noise(seed: u64, secs: f32, amplitude: f32, format: AudioFormat) -> Vec<f32> {
    let n = frame_count(secs, format.sample_rate);
    let mut rng = SplitMix::new(seed);
    let mono: Vec<f32> = (0..n)
        .map(|_| (rng.next_f32() * 2.0 - 1.0) * amplitude)
        .collect();
    interleave(&mono, format.channels)
}

/// A sum of sines at `freqs` for `secs`, scaled so the peak stays within ±0.9.
pub fn chord(freqs: &[f32], secs: f32, format: AudioFormat) -> Vec<f32> {
    let sr = format.sample_rate as f32;
    let n = frame_count(secs, format.sample_rate);
    let scale = if freqs.is_empty() {
        0.0
    } else {
        0.9 / freqs.len() as f32
    };
    let mono: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / sr;
            freqs.iter().map(|f| (TAU * f * t).sin()).sum::<f32>() * scale
        })
        .collect();
    interleave(&mono, format.channels)
}

/// A metronome click track at `bpm` for `secs`: a short decaying broadband burst
/// on each beat, silence between. Fed through the real analyzer it produces an
/// onset (and a beat flag) on each click, ~`60/bpm` seconds apart.
pub fn click_track(bpm: f32, secs: f32, format: AudioFormat) -> Vec<f32> {
    let sr = format.sample_rate as f32;
    let n = frame_count(secs, format.sample_rate);
    let period = ((60.0 / bpm.max(1.0)) * sr).round() as usize;
    let click_len = ((0.012 * sr).round() as usize).max(1); // ~12 ms
    let mut rng = SplitMix::new(0x1234_5678_9ABC_DEF0);
    let mut mono = vec![0.0f32; n];
    let mut start = 0usize;
    while start < n {
        for i in 0..click_len {
            let idx = start + i;
            if idx >= n {
                break;
            }
            let env = (-(i as f32) / click_len as f32 * 6.0).exp();
            let sample = (rng.next_f32() * 2.0 - 1.0) * env * 0.95;
            if let Some(slot) = mono.get_mut(idx) {
                *slot = sample;
            }
        }
        start += period.max(1);
    }
    interleave(&mono, format.channels)
}

/// Interleave a mono buffer up to `channels` (the same sample on every channel).
fn interleave(mono: &[f32], channels: u16) -> Vec<f32> {
    let ch = channels.max(1) as usize;
    let mut out = Vec::with_capacity(mono.len() * ch);
    for &s in mono {
        for _ in 0..ch {
            out.push(s);
        }
    }
    out
}

/// Whole frames in `secs` at `sample_rate` (non-negative).
fn frame_count(secs: f32, sample_rate: u32) -> usize {
    (secs.max(0.0) * sample_rate as f32).round() as usize
}

/// splitmix64 — a tiny seeded PRNG so noise/click generation stays deterministic
/// without a dependency (mirrors the render side's `SeededRng`).
struct SplitMix(u64);

impl SplitMix {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::{Analyzer, HOP_SIZE};

    fn fmt() -> AudioFormat {
        AudioFormat {
            sample_rate: 48_000,
            channels: 2,
        }
    }

    /// Run PCM through the real analyzer, returning the latest frame after the
    /// whole buffer.
    fn analyze_all(pcm: &[f32]) -> crate::dsp::AnalysisFrame {
        let mut an = Analyzer::new(fmt()).expect("valid format");
        an.push_interleaved(pcm);
        an.take_frame()
    }

    #[test]
    fn bass_sine_lands_in_the_bass_band() {
        let frame = analyze_all(&bass_sine(60.0, 1.0, fmt()));
        assert!(
            frame.bass > frame.treb,
            "60 Hz: bass {} should exceed treb {}",
            frame.bass,
            frame.treb
        );
        assert!(frame.bass > 0.05, "60 Hz sine should light the bass band");
    }

    #[test]
    fn treble_tone_lands_in_the_treble_band() {
        let frame = analyze_all(&treble_tone(12_000.0, 1.0, fmt()));
        assert!(
            frame.treb > frame.bass,
            "12 kHz: treb {} should exceed bass {}",
            frame.treb,
            frame.bass
        );
    }

    #[test]
    fn click_track_produces_periodic_onsets() {
        let format = fmt();
        let pcm = click_track(120.0, 3.0, format); // 120 BPM => 0.5 s apart
        let mut an = Analyzer::new(format).expect("valid format");
        let hop = HOP_SIZE * format.channels as usize;
        let secs_per_frame = HOP_SIZE as f32 / format.sample_rate as f32;

        let mut beat_secs = Vec::new();
        for (frame, chunk) in pcm.chunks(hop).enumerate() {
            an.push_interleaved(chunk);
            if an.take_frame().beat {
                beat_secs.push(frame as f32 * secs_per_frame);
            }
        }

        // ~6 beats over 3 s (allow warm-up to swallow the first, and slack).
        assert!(
            (4..=7).contains(&beat_secs.len()),
            "expected ~6 beats over 3 s, got {}: {beat_secs:?}",
            beat_secs.len()
        );
        // Consecutive beats sit near 0.5 s apart.
        for pair in beat_secs.windows(2) {
            let gap = pair[1] - pair[0];
            assert!(
                (0.35..=0.65).contains(&gap),
                "beat gap {gap:.3}s should be ~0.5s (beats {beat_secs:?})"
            );
        }
    }
}
