//! Plan 0001 Phase 3 fixtures: known signals in, expected analysis out.

use lmv_core::audio::AudioFormat;
use lmv_core::dsp::{Analyzer, HOP_SIZE, SPECTRUM_BINS, WINDOW_SIZE};

const SR: u32 = 48_000;

fn mono_analyzer() -> Analyzer {
    Analyzer::new(AudioFormat {
        sample_rate: SR,
        channels: 1,
    })
    .expect("valid format")
}

fn sine(freq: f32, amp: f32, len: usize) -> Vec<f32> {
    (0..len)
        .map(|i| amp * (std::f32::consts::TAU * freq * i as f32 / SR as f32).sin())
        .collect()
}

/// Click track: near-silence with a short 0.9-amplitude burst at every beat.
fn click_track(period_samples: usize, len: usize) -> Vec<f32> {
    let mut signal = vec![0.0f32; len];
    let mut pos = period_samples;
    while pos + 32 < len {
        for (i, s) in signal[pos..pos + 32].iter_mut().enumerate() {
            // Alternating-sign burst: broadband, deterministic, no RNG.
            *s = if i % 2 == 0 { 0.9 } else { -0.9 };
        }
        pos += period_samples;
    }
    signal
}

#[test]
fn sine_energy_concentrates_in_expected_band() {
    let mut analyzer = mono_analyzer();
    let freq = 1_000.0;
    let signal = sine(freq, 0.8, SR as usize);
    analyzer.push_interleaved(&signal);
    let frame = analyzer.take_frame();

    let expected = analyzer.band_for_freq(freq);
    let peak_band = frame
        .spectrum
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(k, _)| k)
        .expect("spectrum is non-empty");
    assert_eq!(
        peak_band, expected,
        "energy should peak in the band containing {freq} Hz"
    );

    // Normalization: a 0.8-amplitude sine should read near 0.8 in its band.
    let peak = frame.spectrum[expected];
    assert!(
        (0.55..=1.0).contains(&peak),
        "peak band value {peak} should be near the sine amplitude 0.8"
    );

    // The energy is concentrated: away from the peak's immediate neighbors
    // (Hann leakage), every band stays far below the peak.
    for (k, &v) in frame.spectrum.iter().enumerate() {
        if k.abs_diff(expected) > 1 {
            assert!(
                v < 0.1 * peak,
                "band {k} = {v} should be well below the {freq} Hz peak {peak}"
            );
        }
    }
}

#[test]
fn click_track_produces_onsets_on_the_beats() {
    let mut analyzer = mono_analyzer();
    // 120 BPM at 48 kHz: a click every 24000 samples, 5 seconds of signal.
    let period = 24_000usize;
    let signal = click_track(period, 5 * SR as usize);

    let mut beat_hops = Vec::new();
    for (hop_idx, hop) in signal.chunks_exact(HOP_SIZE).enumerate() {
        analyzer.push_interleaved(hop);
        if analyzer.take_frame().beat {
            beat_hops.push(hop_idx);
        }
    }

    // Clicks start at `period` and repeat while a full burst fits.
    let expected_clicks: Vec<usize> = (1..)
        .map(|k| k * period)
        .take_while(|&pos| pos + 32 < signal.len())
        .collect();
    assert_eq!(expected_clicks.len(), 9, "fixture sanity");

    // Every click produces exactly one beat within 3 hops of the hop that
    // first contains it, and there are no spurious beats elsewhere.
    let tolerance = 3;
    for &click_pos in &expected_clicks {
        let click_hop = click_pos / HOP_SIZE;
        let matches = beat_hops
            .iter()
            .filter(|&&h| h.abs_diff(click_hop) <= tolerance)
            .count();
        assert_eq!(
            matches, 1,
            "click at hop {click_hop} should produce exactly one beat, got {beat_hops:?}"
        );
    }
    assert_eq!(
        beat_hops.len(),
        expected_clicks.len(),
        "no spurious beats: {beat_hops:?}"
    );
}

#[test]
fn analysis_is_deterministic() {
    let signal = {
        let mut s = sine(440.0, 0.5, 2 * SR as usize);
        let clicks = click_track(12_000, s.len());
        for (a, b) in s.iter_mut().zip(clicks.iter()) {
            *a += b;
        }
        s
    };

    let run = |mut analyzer: Analyzer| -> Vec<(Vec<u32>, u32, bool)> {
        signal
            .chunks_exact(HOP_SIZE)
            .map(|hop| {
                analyzer.push_interleaved(hop);
                let f = analyzer.take_frame();
                // Bit-exact comparison via raw f32 bits.
                (
                    f.spectrum.iter().map(|v| v.to_bits()).collect(),
                    f.onset.to_bits(),
                    f.beat,
                )
            })
            .collect()
    };

    assert_eq!(run(mono_analyzer()), run(mono_analyzer()));
}

#[test]
fn one_hop_analyzes_well_under_the_hop_interval() {
    let mut analyzer = mono_analyzer();
    let signal = sine(440.0, 0.5, WINDOW_SIZE + 1000 * HOP_SIZE);
    analyzer.push_interleaved(&signal[..WINDOW_SIZE]);

    let start = std::time::Instant::now();
    for hop in signal[WINDOW_SIZE..].chunks_exact(HOP_SIZE) {
        analyzer.push_interleaved(hop);
    }
    let per_hop = start.elapsed() / 1000;

    // Hop interval is ~10.7 ms at 48 kHz; even unoptimized builds should sit
    // far below it (NFR section 3 / plan done-when).
    assert!(
        per_hop < std::time::Duration::from_millis(11),
        "one hop took {per_hop:?}, budget is ~11 ms"
    );
    // Sanity that the spectrum output stayed meaningful end-to-end.
    assert!(analyzer.take_frame().spectrum.iter().sum::<f32>() > 0.0);
    assert_eq!(SPECTRUM_BINS, 64);
}
