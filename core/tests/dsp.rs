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

/// Sum of equal-amplitude sines (a chord), scaled so the peak stays within
/// `amp`. Two chords on disjoint frequency ranges give clearly distinct spectra.
fn chord(freqs: &[f32], amp: f32, len: usize) -> Vec<f32> {
    let scale = if freqs.is_empty() {
        0.0
    } else {
        amp / freqs.len() as f32
    };
    (0..len)
        .map(|i| {
            let t = i as f32 / SR as f32;
            freqs
                .iter()
                .map(|f| (std::f32::consts::TAU * f * t).sin())
                .sum::<f32>()
                * scale
        })
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
fn tempo_estimate_locks_onto_a_known_click_train() {
    let mut analyzer = mono_analyzer();
    // 120 BPM at 48 kHz: a click every 24000 samples. 12 s is well past the
    // tempo tracker's ~4 s envelope-history warmup.
    let period = 24_000usize;
    let signal = click_track(period, 12 * SR as usize);
    analyzer.push_interleaved(&signal);
    let bpm = analyzer.take_frame().bpm;

    // Hop-clock autocorrelation should land the tempo within a few BPM of the
    // true 120 (a determinism/correctness claim, not "the test runs").
    assert!(
        (bpm - 120.0).abs() <= 3.0,
        "estimated tempo {bpm} should be within 3 BPM of the 120 BPM click train"
    );
}

#[test]
fn band_split_is_frequency_correct() {
    // A pure low tone lands its energy in bass with ~none in treble...
    let mut low_an = mono_analyzer();
    low_an.push_interleaved(&sine(60.0, 0.8, SR as usize));
    let low = low_an.take_frame();
    assert!(
        low.bass > 0.01 && low.bass > low.mid && low.bass > low.treb,
        "60 Hz energy should dominate the bass band (bass={}, mid={}, treb={})",
        low.bass,
        low.mid,
        low.treb
    );
    assert!(
        low.treb < 0.1 * low.bass,
        "treble should be near-empty for a 60 Hz tone (treb={}, bass={})",
        low.treb,
        low.bass
    );

    // ...and the mirror holds for a pure high tone.
    let mut high_an = mono_analyzer();
    high_an.push_interleaved(&sine(6_000.0, 0.8, SR as usize));
    let high = high_an.take_frame();
    assert!(
        high.treb > 0.0 && high.treb > high.bass && high.treb > high.mid,
        "6 kHz energy should dominate the treble band (bass={}, mid={}, treb={})",
        high.bass,
        high.mid,
        high.treb
    );
    assert!(
        high.bass < 0.1 * high.treb,
        "bass should be near-empty for a 6 kHz tone (bass={}, treb={})",
        high.bass,
        high.treb
    );
}

#[test]
fn novelty_spikes_at_a_spectral_boundary() {
    let mut analyzer = mono_analyzer();
    // Two 3 s segments with disjoint spectra: a low chord then a high chord.
    let seg = 3 * SR as usize;
    let mut signal = chord(&[110.0, 220.0, 330.0], 0.8, seg);
    signal.extend_from_slice(&chord(&[4_000.0, 6_000.0, 8_000.0], 0.8, seg));

    let boundary_hop = seg / HOP_SIZE;
    let novelty: Vec<f32> = signal
        .chunks_exact(HOP_SIZE)
        .map(|hop| {
            analyzer.push_interleaved(hop);
            analyzer.take_frame().novelty
        })
        .collect();

    // Well inside segment A the spectrum sits on its own running mean: ~0.
    let steady_a = novelty[boundary_hop / 2];
    // Late in segment B the mean has caught up to the new spectrum: also low.
    let steady_b = *novelty.last().expect("frames were produced");
    // The boundary (plus the ~4-hop window transition) spikes.
    let peak = novelty[boundary_hop..(boundary_hop + 12).min(novelty.len())]
        .iter()
        .copied()
        .fold(0.0f32, f32::max);

    assert!(
        steady_a < 0.1,
        "steady segment-A novelty {steady_a} should be near zero"
    );
    assert!(peak > 0.4, "the boundary should spike novelty (got {peak})");
    assert!(
        peak > steady_a * 5.0 + 0.2,
        "boundary spike {peak} should stand out from steady {steady_a}"
    );
    assert!(
        steady_b < peak,
        "late segment-B novelty {steady_b} should decay below the spike {peak}"
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

    let run = |mut analyzer: Analyzer| -> Vec<(Vec<u32>, u32, bool, u32, u32, u32)> {
        signal
            .chunks_exact(HOP_SIZE)
            .map(|hop| {
                analyzer.push_interleaved(hop);
                let f = analyzer.take_frame();
                // Bit-exact comparison via raw f32 bits — covers the enriched
                // bpm/bar and novelty too, so those paths are proven deterministic.
                (
                    f.spectrum.iter().map(|v| v.to_bits()).collect(),
                    f.onset.to_bits(),
                    f.beat,
                    f.bpm.to_bits(),
                    f.bar.to_bits(),
                    f.novelty.to_bits(),
                )
            })
            .collect()
    };

    assert_eq!(run(mono_analyzer()), run(mono_analyzer()));
}

#[test]
#[allow(
    clippy::disallowed_methods,
    reason = "perf smoke test deliberately times execution; the analysis under test stays clock-free"
)]
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
