//! Shape sanity (Plan 0013 Phase 3, HARD). A newly-added scene that drew nothing
//! or a single dot should fail before it ships. Under a sustained *loud* frame
//! (so audio-gated brightness is up), assert each preset lights a minimum
//! fraction of the frame (`coverage`) and spreads across at least two quadrants
//! (`quadrant_spread`) — "not blank, not a dot".
//!
//! The background is sampled from a corner pixel, **not** assumed to be black:
//! `fragment_field` clears to black but `swarm` clears to a dark blue, so a
//! fixed black background would score every swarm frame as fully lit (a
//! tautology — Plan 0013 Risks). Measuring foreground against the frame's own
//! background makes a blank frame score 0 whatever colour it cleared to.
//!
//! Coverage floors are per-system: `fragment_field` fills the frame, while the
//! `swarm` is sparse points, so a single broad floor would be either tautological
//! for one or impossible for the other.

use lmv_core::dsp::AnalysisFrame;
use lmv_core::preset::{SystemKind, default_presets};
use lmv_core::render::{
    CaptureImage, HeadlessOptions, RenderError, Renderer,
    metrics::{coverage, quadrant_spread},
};

const SIZE: u32 = 96;
const FRAMES: u32 = 30;
/// A pixel counts as lit if any RGB channel differs from the sampled background
/// by more than this (shrugs off dark near-background dithering).
const EPS: u8 = 10;
/// Minimum lit quadrants — a dot in one corner fails.
const MIN_QUADRANTS: u8 = 2;

/// Per-system minimum lit fraction. The full-screen field must fill most of the
/// frame; the sparse swarm need only paint a small but real footprint.
fn coverage_floor(system: SystemKind) -> f32 {
    match system {
        SystemKind::FragmentField => 0.30,
        // Sparse line art / point swarm: a small but real footprint.
        SystemKind::Swarm
        | SystemKind::ParametricCurve
        | SystemKind::LSystem
        | SystemKind::StarPattern => 0.01,
    }
}

fn system_name(system: SystemKind) -> &'static str {
    match system {
        SystemKind::FragmentField => "fragment_field",
        SystemKind::Swarm => "swarm",
        SystemKind::ParametricCurve => "parametric_curve",
        SystemKind::LSystem => "lsystem",
        SystemKind::StarPattern => "star_pattern",
    }
}

/// Build a headless `Renderer`, or `None` (a logged skip) when the runner
/// exposes no GPU adapter — macOS has no software Metal fallback (ADR-0016).
/// Any other build error still panics loudly.
fn headless() -> Option<Renderer> {
    match Renderer::new_headless(HeadlessOptions {
        width: SIZE,
        height: SIZE,
        prefer_software: true,
    }) {
        Ok(r) => Some(r),
        Err(RenderError::RequestAdapter(_)) => {
            eprintln!("skipped: no GPU adapter on this runner (ADR-0016)");
            None
        }
        Err(e) => panic!("headless renderer build failed: {e}"),
    }
}

/// A sustained "loud" frame: every band up and a beat, so any audio-gated
/// brightness reaches its lit state.
fn loud() -> AnalysisFrame {
    AnalysisFrame {
        bass: 1.0,
        mid: 1.0,
        treb: 1.0,
        onset: 1.0,
        beat: true,
        bar: 0.5,
        ..Default::default()
    }
}

/// The top-left pixel, taken as the scene's background colour (the clear colour
/// each built-in scene paints its corners with).
fn background(img: &CaptureImage) -> [u8; 4] {
    [
        img.rgba.first().copied().unwrap_or(0),
        img.rgba.get(1).copied().unwrap_or(0),
        img.rgba.get(2).copied().unwrap_or(0),
        img.rgba.get(3).copied().unwrap_or(255),
    ]
}

#[test]
fn every_preset_draws_a_real_shape() {
    let Some(mut renderer) = headless() else {
        return;
    };
    let frame = loud();

    let mut failures = Vec::new();
    for preset in default_presets() {
        let img = renderer
            .capture_preset(&preset.name, &frame, FRAMES)
            .expect("capture preset");
        let bg = background(&img);
        let cov = coverage(&img, bg, EPS);
        let spread = quadrant_spread(&img, bg, EPS);
        let floor = coverage_floor(preset.system);
        println!(
            "[{}] {:<12} coverage={cov:.4} (floor {floor:.2}) quadrants={spread}",
            system_name(preset.system),
            preset.name,
        );
        if cov < floor {
            failures.push(format!(
                "{} blank: coverage {cov:.4} < {floor:.2}",
                preset.name
            ));
        }
        if spread < MIN_QUADRANTS {
            failures.push(format!(
                "{} is a dot: {spread} quadrant(s) < {MIN_QUADRANTS}",
                preset.name
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "these presets failed shape sanity: {failures:#?}"
    );
}
