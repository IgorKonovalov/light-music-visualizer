//! Golden-image drift (Plan 0013 Phase 4; repointed by Plan 0022, ADR-0023).
//! This guard defends **engine rendering determinism**, not shipped content: it
//! renders one **frozen per-system fixture** headless on the software adapter
//! and compares each frame against a committed baseline PNG within a mean +
//! max-outlier tolerance. `LMV_BLESS=1` rewrites the baselines.
//!
//! The fixtures live as do-not-tune TOML under `tests/fixtures/`, one per
//! [`SystemKind`], selected by an **exhaustive** match (see [`fixture`]) so a new
//! scene cannot ship without a drift baseline. They are deliberately *not* the
//! shipped presets: the `preset-author` lane (ADR-0017) tunes those, and an
//! intended content tune must never trip this engine-drift alarm. Shipped
//! presets keep their own guards behaviorally — `sanity` (coverage/spread),
//! `reactivity` (per-band reaction), and `animation` (motion) each iterate
//! `default_presets()`, and those floors survive content tuning by design.
//!
//! The tolerance absorbs cross-GPU rasterization drift (the software adapter
//! keeps it small); a genuine engine change — a perturbed shader or scene math —
//! moves a frame well past it. Baselines are ordinary PNGs, viewable in the repo
//! and PR diffs; they are WARP-only (macOS has no software Metal fallback, so
//! the test skips there per ADR-0016) and must be blessed on WARP. Eyeball each
//! before blessing (Plan 0013 Phase 8 habit).

use std::path::{Path, PathBuf};

use lmv_core::dsp::AnalysisFrame;
use lmv_core::preset::{Preset, SystemKind};
use lmv_core::render::{CaptureImage, HeadlessOptions, RenderError, Renderer, metrics::frame_diff};

const SIZE: u32 = 128;
/// Frames warmed before capture — enough for the stateful systems (swarm sim,
/// reaction-diffusion field) to evolve a non-trivial pattern.
const FRAMES: u32 = 60;
/// Mean per-channel difference (0..1) a fresh render may drift from baseline.
const MEAN_TOL: f32 = 0.02;
/// Largest single-channel byte difference tolerated at any pixel — a localized
/// change a low mean would otherwise hide.
const MAX_OUTLIER: u8 = 48;

/// Every `SystemKind` the drift guard must cover. Iterating this list drives the
/// test; the exhaustive `match` in [`fixture`] is what forces a new variant to
/// add its fixture before the test compiles.
const SYSTEMS: &[SystemKind] = &[
    SystemKind::FragmentField,
    SystemKind::Swarm,
    SystemKind::ParametricCurve,
    SystemKind::LSystem,
    SystemKind::StarPattern,
    SystemKind::ReactionDiffusion,
    SystemKind::Attractor,
];

/// The frozen fixture for a system: its baseline file stem (the system name) and
/// the fixture TOML compiled into the test binary.
///
/// This is an **exhaustive** `match` with no wildcard arm — adding a
/// `SystemKind` variant fails to compile here until a fixture is authored under
/// `tests/fixtures/`, so no scene ships without a drift baseline (ADR-0023).
fn fixture(system: SystemKind) -> (&'static str, &'static str) {
    match system {
        SystemKind::FragmentField => (
            "fragment_field",
            include_str!("fixtures/fragment_field.toml"),
        ),
        SystemKind::Swarm => ("swarm", include_str!("fixtures/swarm.toml")),
        SystemKind::ParametricCurve => (
            "parametric_curve",
            include_str!("fixtures/parametric_curve.toml"),
        ),
        SystemKind::LSystem => ("lsystem", include_str!("fixtures/lsystem.toml")),
        SystemKind::StarPattern => ("star_pattern", include_str!("fixtures/star_pattern.toml")),
        SystemKind::ReactionDiffusion => (
            "reaction_diffusion",
            include_str!("fixtures/reaction_diffusion.toml"),
        ),
        SystemKind::Attractor => ("attractor", include_str!("fixtures/attractor.toml")),
    }
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
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

/// The fixed frame every baseline is rendered under — a representative
/// mid-energy frame with all bands lit, so a band-reactive fixture still draws.
fn fixed_frame() -> AnalysisFrame {
    AnalysisFrame {
        bass: 0.6,
        mid: 0.5,
        treb: 0.6,
        onset: 0.4,
        bar: 0.25,
        ..Default::default()
    }
}

fn decode(path: &Path) -> CaptureImage {
    let img = image::open(path)
        .unwrap_or_else(|e| panic!("decode baseline {}: {e}", path.display()))
        .to_rgba8();
    CaptureImage {
        width: img.width(),
        height: img.height(),
        rgba: img.into_raw(),
    }
}

fn encode(img: &CaptureImage, path: &Path) {
    let buffer = image::RgbaImage::from_raw(img.width, img.height, img.rgba.clone())
        .expect("capture buffer matches its declared dimensions");
    buffer
        .save(path)
        .unwrap_or_else(|e| panic!("write baseline {}: {e}", path.display()));
}

/// Largest absolute single-channel (RGB) byte difference across the two images.
fn max_channel_outlier(a: &CaptureImage, b: &CaptureImage) -> u8 {
    a.rgba
        .chunks_exact(4)
        .zip(b.rgba.chunks_exact(4))
        .flat_map(|(pa, pb)| {
            pa.iter()
                .zip(pb.iter())
                .take(3)
                .map(|(x, y)| x.abs_diff(*y))
        })
        .max()
        .unwrap_or(0)
}

#[test]
fn scenes_match_golden_baselines() {
    let Some(mut renderer) = headless() else {
        return;
    };
    let frame = fixed_frame();
    let bless = std::env::var_os("LMV_BLESS").is_some();
    std::fs::create_dir_all(golden_dir()).expect("create tests/golden");

    let mut failures = Vec::new();
    for &system in SYSTEMS {
        let (stem, toml) = fixture(system);
        let preset = Preset::from_toml_str(toml)
            .unwrap_or_else(|e| panic!("golden fixture {stem}.toml is invalid: {e}"));
        let name = preset.name.clone();
        renderer.set_presets(vec![preset]);

        let fresh = renderer
            .capture_preset(&name, &frame, FRAMES)
            .expect("capture fixture");
        let path = golden_dir().join(format!("{stem}.png"));

        if bless {
            encode(&fresh, &path);
            println!("blessed {}", path.display());
            continue;
        }

        assert!(
            path.exists(),
            "missing baseline {} — run `LMV_BLESS=1 cargo test -p lmv-core --test golden`",
            path.display()
        );
        let baseline = decode(&path);
        let mean = frame_diff(&baseline, &fresh);
        let outlier = max_channel_outlier(&baseline, &fresh);
        println!(
            "{stem:<18} mean {mean:.4} (tol {MEAN_TOL}) max_outlier {outlier} (tol {MAX_OUTLIER})"
        );
        if mean > MEAN_TOL || outlier > MAX_OUTLIER {
            failures.push(format!(
                "{stem}: mean {mean:.4} / outlier {outlier} exceeds tolerance"
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "golden drift beyond tolerance (bless with LMV_BLESS=1 if intended): {failures:#?}"
    );
}
