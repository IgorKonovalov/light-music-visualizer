//! Animation liveness (Plan 0013 Phase 3, HARD). A scene must change over time
//! independent of audio: hold the audio frame constant and compare frame N with
//! frame N+k. A frozen scene (e.g. `time` unbound, or a stuck clock) reads as
//! near-zero and fails. Silent audio is used deliberately — the motion under
//! test is the shared scene clock, not an audio edge.
//!
//! Software adapter so it holds on any CI GPU.

use lmv_core::dsp::AnalysisFrame;
use lmv_core::preset::{SystemKind, default_presets};
use lmv_core::render::{HeadlessOptions, Renderer, metrics::frame_diff};

const SIZE: u32 = 96;
/// The earlier and later capture points (frames). The ~0.4 s gap at the fixed
/// 60 fps `SCENE_DT` is ample for any live scene to move visibly.
const FRAME_A: u32 = 24;
const FRAME_B: u32 = 48;
/// Minimum motion (mean-abs RGB, 0..1) between the two frames. Catches a
/// *frozen* scene, not a subtly-animated one.
const ANIM_FLOOR: f32 = 0.01;

fn system_name(system: SystemKind) -> &'static str {
    match system {
        SystemKind::FragmentField => "fragment_field",
        SystemKind::Swarm => "swarm",
    }
}

fn headless() -> Renderer {
    Renderer::new_headless(HeadlessOptions {
        width: SIZE,
        height: SIZE,
        prefer_software: true,
    })
    .expect("headless renderer builds on the software adapter")
}

#[test]
fn every_preset_animates_over_time() {
    let mut renderer = headless();
    let audio = AnalysisFrame::default();

    let mut failures = Vec::new();
    for preset in default_presets() {
        let early = renderer
            .capture_preset(&preset.name, &audio, FRAME_A)
            .expect("capture early frame");
        let late = renderer
            .capture_preset(&preset.name, &audio, FRAME_B)
            .expect("capture late frame");
        let motion = frame_diff(&early, &late);
        println!(
            "[{}] {:<12} frame {FRAME_A} vs {FRAME_B}: {motion:.4}",
            system_name(preset.system),
            preset.name,
        );
        if motion < ANIM_FLOOR {
            failures.push(format!("{} (motion {motion:.4})", preset.name));
        }
    }

    assert!(
        failures.is_empty(),
        "these presets do not animate above {ANIM_FLOOR}: {failures:#?}"
    );
}
