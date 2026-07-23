//! GPU compute-particle attractor contract (Plan 0016 Phase 5, HARD). The
//! attractor scene is the engine's first *compute pipeline* + GPU-resident
//! particle system, so beyond the generic per-preset gates (sanity / animation /
//! reactivity, which already include the four shipped attractor presets) it gets
//! a focused suite here — most importantly a **seed reproducibility** check (the
//! Phase 1 determinism done-when) and a **beat perturbation** check (Phase 3), the
//! two properties the generic differential loops don't assert directly.
//!
//! All checks ride Plan 0013's `capture_preset`, which rebuilds the scene to its
//! seed and resets the clock, so a capture is a pure function of `(preset, frame,
//! frames)` under the fixed capture `dt`. Software adapter (`prefer_software`) so
//! it holds on any CI GPU and reproduces bit-for-bit.
//!
//! The checks share one renderer in a single `#[test]` (one per file, like the
//! other GPU suites): distinct headless renderers built in parallel each spin up
//! a WARP device and can crash the software driver.

use lmv_core::dsp::AnalysisFrame;
use lmv_core::render::{
    CaptureImage, HeadlessOptions, RenderError, Renderer,
    metrics::{coverage, frame_diff, quadrant_spread},
};

const SIZE: u32 = 96;
/// The 2D map preset (De Jong) and a 3D flow preset (Lorenz) from the embedded
/// set — one of each idiom the scene supports.
const DEJONG: &str = "De Jong";
const LORENZ: &str = "Lorenz";
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

/// The top-left pixel, taken as the scene's background colour (the near-black bed
/// the attractor clears its trail field to).
fn background(img: &CaptureImage) -> [u8; 4] {
    [
        img.rgba.first().copied().unwrap_or(0),
        img.rgba.get(1).copied().unwrap_or(0),
        img.rgba.get(2).copied().unwrap_or(0),
        img.rgba.get(3).copied().unwrap_or(255),
    ]
}

#[test]
fn attractor_contract() {
    let Some(mut renderer) = headless() else {
        return;
    };

    // A sustained mid-energy frame; no beat.
    let lively = AnalysisFrame {
        bass: 0.5,
        mid: 0.4,
        treb: 0.5,
        ..Default::default()
    };

    // --- Shape sanity: the De Jong cloud is neither blank nor a single dot. ---
    let warm = renderer
        .capture_preset(DEJONG, &lively, 60)
        .expect("capture De Jong @60");
    let bg = background(&warm);
    let cov = coverage(&warm, bg, EPS);
    let spread = quadrant_spread(&warm, bg, EPS);
    assert!(cov > 0.02, "De Jong cloud is blank: coverage {cov:.4}");
    assert!(spread >= 2, "De Jong cloud is a dot: {spread} quadrant(s)");

    // --- Seed reproducibility (Phase 1 determinism done-when): the seeded init +
    // deterministic compute step reproduce bit-for-bit on the same adapter — the
    // property a GPU-resident particle sim most easily loses. ---
    let a = renderer
        .capture_preset(DEJONG, &lively, 48)
        .expect("capture A");
    let b = renderer
        .capture_preset(DEJONG, &lively, 48)
        .expect("capture B");
    assert_eq!(
        a.rgba, b.rgba,
        "attractor capture is not reproducible for a fixed input"
    );

    // --- Animation: a later frame differs from an earlier one (boiling + spin +
    // trails move it), not frozen. ---
    let early = renderer
        .capture_preset(DEJONG, &lively, 24)
        .expect("capture @24");
    let motion = frame_diff(&early, &warm);
    assert!(motion > 0.01, "attractor is frozen: motion {motion:.4}");

    // --- Beat perturbation (Phase 3): a beat re-scatters the cloud and swells the
    // points, so a beat frame differs from an otherwise-identical calm one. ---
    let calm = AnalysisFrame {
        bass: 0.3,
        mid: 0.3,
        ..Default::default()
    };
    let beat = AnalysisFrame { beat: true, ..calm };
    let without = renderer
        .capture_preset(DEJONG, &calm, 60)
        .expect("capture calm");
    let with = renderer
        .capture_preset(DEJONG, &beat, 60)
        .expect("capture beat");
    let delta = frame_diff(&without, &with);
    assert!(delta > 0.003, "beat did not perturb the cloud: {delta:.4}");

    // --- 3D flow: the Lorenz butterfly renders a real shape, exercising the
    // continuous-family compute path (Euler integration + 3D projection). ---
    let lorenz = renderer
        .capture_preset(LORENZ, &lively, 90)
        .expect("capture Lorenz @90");
    let lbg = background(&lorenz);
    let lcov = coverage(&lorenz, lbg, EPS);
    let lspread = quadrant_spread(&lorenz, lbg, EPS);
    assert!(lcov > 0.02, "Lorenz flow is blank: coverage {lcov:.4}");
    assert!(lspread >= 2, "Lorenz flow is a dot: {lspread} quadrant(s)");
}
