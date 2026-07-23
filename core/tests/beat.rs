//! Beat response through the real DSP (Plan 0013 Phase 6, HARD). A synthetic
//! 120 BPM click track is fed through the *real* analyzer (FFT → onset → beat)
//! by `capture_audio`; a beat-accent preset must render visibly differently on a
//! beat frame than on a nearby non-beat frame. A constant hand-set frame could
//! never test this — the beat has to actually fire.
//!
//! The probe is baked in: a second preset with its beat binding **zeroed** must
//! *not* show the response between the same two frames, proving the difference
//! comes from the beat binding and not from time drift.
//!
//! Beat frame indices are found by running an analyzer over the click track the
//! same way `capture_audio` feeds it (hop-by-hop), so the indices line up
//! deterministically without depending on the tempo lock.

use lmv_core::audio::AudioFormat;
use lmv_core::dsp::{Analyzer, HOP_SIZE};
use lmv_core::preset::Preset;
use lmv_core::render::{HeadlessOptions, RenderError, Renderer, metrics::frame_diff};
use lmv_core::signal::click_track;

const SIZE: u32 = 96;
const BPM: f32 = 120.0;
const SECS: f32 = 4.0;
/// On-beat vs off-beat must differ at least this much for the live preset, and
/// the dead-binding preset must stay below it. Comfortably separates the two.
const BEAT_FLOOR: f32 = 0.03;

fn fmt() -> AudioFormat {
    AudioFormat {
        sample_rate: 48_000,
        channels: 2,
    }
}

/// A minimal fragment_field preset whose whole look is driven by `beat`.
fn live_preset() -> Preset {
    Preset::from_toml_str(
        "system = \"fragment_field\"\n\
         name = \"beat_live\"\n\
         [params]\n\
         flash = \"beat\"\n\
         glow  = \"0.15 + beat * 0.85\"\n\
         warp  = \"0.4 + beat * 1.6\"",
    )
    .expect("hand-written beat preset is valid")
}

/// The same preset with every `beat` reference replaced by a constant — the
/// zeroed-binding probe.
fn dead_preset() -> Preset {
    Preset::from_toml_str(
        "system = \"fragment_field\"\n\
         name = \"beat_dead\"\n\
         [params]\n\
         flash = \"0\"\n\
         glow  = \"0.15\"\n\
         warp  = \"0.4\"",
    )
    .expect("hand-written dead preset is valid")
}

/// The per-frame beat flags produced by feeding `pcm` hop-by-hop — the same
/// cadence `capture_audio` uses, so indices align.
fn beat_flags(pcm: &[f32], format: AudioFormat) -> Vec<bool> {
    let mut analyzer = Analyzer::new(format).expect("valid format");
    let hop = HOP_SIZE * format.channels as usize;
    pcm.chunks(hop)
        .map(|chunk| {
            analyzer.push_interleaved(chunk);
            analyzer.take_frame().beat
        })
        .collect()
}

#[test]
fn beat_accent_preset_responds_on_beat() {
    let format = fmt();
    let pcm = click_track(BPM, SECS, format);
    let flags = beat_flags(&pcm, format);

    let beats: Vec<usize> = flags
        .iter()
        .enumerate()
        .filter(|(_, b)| **b)
        .map(|(i, _)| i)
        .collect();
    assert!(
        beats.len() >= 3,
        "the click track should fire several beats, got {beats:?}"
    );

    // A beat frame (skip the first in case of analyzer warm-up), and the nearest
    // non-beat frame a few hops later (small time drift).
    let beat_idx = beats[1];
    let mut between = beat_idx + 4;
    while between < flags.len() && flags[between] {
        between += 1;
    }
    assert!(between < flags.len(), "a non-beat frame follows the beat");
    let at = [beat_idx as u32, between as u32];

    // No GPU adapter (macOS has no software Metal fallback) is a logged skip,
    // not a failure (ADR-0016); any other build error still panics loudly.
    let mut renderer = match Renderer::new_headless(HeadlessOptions {
        width: SIZE,
        height: SIZE,
        prefer_software: true,
    }) {
        Ok(r) => r,
        Err(RenderError::RequestAdapter(_)) => {
            eprintln!("skipped: no GPU adapter on this runner (ADR-0016)");
            return;
        }
        Err(e) => panic!("headless renderer build failed: {e}"),
    };

    renderer.set_presets(vec![live_preset()]);
    let live = renderer
        .capture_audio("beat_live", &pcm, format, &at)
        .expect("capture live preset");
    let live_diff = frame_diff(&live[0], &live[1]);

    renderer.set_presets(vec![dead_preset()]);
    let dead = renderer
        .capture_audio("beat_dead", &pcm, format, &at)
        .expect("capture dead preset");
    let dead_diff = frame_diff(&dead[0], &dead[1]);

    println!(
        "beat_idx={beat_idx} between={between} live_diff={live_diff:.4} dead_diff={dead_diff:.4}"
    );
    assert!(
        live_diff >= BEAT_FLOOR,
        "beat-accent preset must respond on-beat (live {live_diff:.4} < floor {BEAT_FLOOR})"
    );
    assert!(
        dead_diff < BEAT_FLOOR,
        "zeroing the beat binding removes the response (dead {dead_diff:.4} >= floor {BEAT_FLOOR})"
    );
}
