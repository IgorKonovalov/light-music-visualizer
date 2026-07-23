//! Reaction-diffusion scene contract (Plan 0014 Phase 6, HARD). The RD scene is
//! the engine's first *stateful feedback* system, so beyond the generic
//! per-preset gates (sanity / animation / reactivity, which already include
//! Coral) it gets a focused suite here — most importantly a **seed
//! reproducibility** check, the property a running simulation most easily
//! breaks (ADR-0012).
//!
//! All four checks ride Plan 0013's `capture_preset`, which rebuilds the scene
//! to its seed and resets the clock, so a capture is a pure function of
//! `(preset, frame, frames)` under the fixed capture `dt`. Software adapter
//! (`prefer_software`) so it holds on any CI GPU and reproduces bit-for-bit.
//!
//! The four checks share one renderer in a single `#[test]` (one per file, like
//! the other GPU suites): distinct headless renderers built in parallel each
//! spin up a WARP device and can crash the software driver.

use lmv_core::dsp::AnalysisFrame;
use lmv_core::render::{
    CaptureImage, HeadlessOptions, RenderError, Renderer,
    metrics::{coverage, frame_diff, quadrant_spread},
};

const SIZE: u32 = 96;
/// The RD preset shipped in the embedded set.
const PRESET: &str = "Coral";
/// A pixel counts as lit if any RGB channel differs from the sampled background
/// by more than this.
const EPS: u8 = 10;

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

/// The top-left pixel, taken as the scene's background colour.
fn background(img: &CaptureImage) -> [u8; 4] {
    [
        img.rgba.first().copied().unwrap_or(0),
        img.rgba.get(1).copied().unwrap_or(0),
        img.rgba.get(2).copied().unwrap_or(0),
        img.rgba.get(3).copied().unwrap_or(255),
    ]
}

#[test]
fn reaction_diffusion_contract() {
    let Some(mut renderer) = headless() else {
        return;
    };

    // A sustained mid-energy frame that keeps the field lively; no beat.
    let lively = AnalysisFrame {
        bass: 0.5,
        mid: 0.3,
        treb: 0.3,
        ..Default::default()
    };

    // --- Shape sanity: a warmed field is neither blank nor a single dot. ---
    let warm = renderer
        .capture_preset(PRESET, &lively, 60)
        .expect("capture Coral @60");
    let bg = background(&warm);
    let cov = coverage(&warm, bg, EPS);
    let spread = quadrant_spread(&warm, bg, EPS);
    assert!(cov > 0.03, "field is blank: coverage {cov:.4}");
    assert!(spread >= 2, "field is a dot: {spread} quadrant(s)");

    // --- Animation: a later frame differs from an earlier one (not frozen). ---
    let early = renderer
        .capture_preset(PRESET, &lively, 24)
        .expect("capture @24");
    let motion = frame_diff(&early, &warm);
    assert!(motion > 0.01, "sim is frozen: motion {motion:.4}");

    // --- Reactivity: a beat perturbs the field (stamps a seed of growth). ---
    let calm = AnalysisFrame {
        bass: 0.3,
        mid: 0.3,
        ..Default::default()
    };
    let beat = AnalysisFrame { beat: true, ..calm };
    let without = renderer
        .capture_preset(PRESET, &calm, 60)
        .expect("capture calm");
    let with = renderer
        .capture_preset(PRESET, &beat, 60)
        .expect("capture beat");
    let delta = frame_diff(&without, &with);
    assert!(delta > 0.003, "beat did not perturb the field: {delta:.4}");

    // --- Seed reproducibility (ADR-0012): the stateful sim + seeded injection
    // RNG are deterministic, so the same input reproduces bit-for-bit on the
    // same adapter — the property a running simulation most easily loses. ---
    let repro_frame = AnalysisFrame {
        bass: 0.4,
        mid: 0.3,
        beat: true, // exercise the injection path too
        ..Default::default()
    };
    let a = renderer
        .capture_preset(PRESET, &repro_frame, 48)
        .expect("capture A");
    let b = renderer
        .capture_preset(PRESET, &repro_frame, 48)
        .expect("capture B");
    assert_eq!(
        a.rgba, b.rgba,
        "reaction-diffusion capture is not reproducible for a fixed input"
    );
}
