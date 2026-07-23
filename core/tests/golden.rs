//! Golden-image drift (Plan 0013 Phase 4, HARD with tolerance). Render a small
//! fixed matrix headless on the software adapter and compare each frame against a
//! committed baseline PNG within a mean + max-outlier tolerance. `LMV_BLESS=1`
//! rewrites the baselines.
//!
//! The tolerance absorbs cross-GPU rasterization drift (the software adapter
//! keeps it small); a genuine visual change — a perturbed scene or binding —
//! moves a frame well past it. Baselines are ordinary PNGs, viewable in the repo
//! and PR diffs; eyeball them before blessing (Plan 0013 Phase 8 habit).

use std::path::{Path, PathBuf};

use lmv_core::dsp::AnalysisFrame;
use lmv_core::render::{CaptureImage, HeadlessOptions, RenderError, Renderer, metrics::frame_diff};

const SIZE: u32 = 128;
/// Mean per-channel difference (0..1) a fresh render may drift from baseline.
const MEAN_TOL: f32 = 0.02;
/// Largest single-channel byte difference tolerated at any pixel — a localized
/// change a low mean would otherwise hide.
const MAX_OUTLIER: u8 = 48;

struct Case {
    /// Baseline file stem under `tests/golden/`.
    file: &'static str,
    /// Preset name in the default roster.
    preset: &'static str,
    frames: u32,
}

/// A small matrix spanning both systems (two fragment_field, one swarm).
const CASES: &[Case] = &[
    Case {
        file: "aurora",
        preset: "Aurora",
        frames: 60,
    },
    Case {
        file: "warp",
        preset: "Warp Drive",
        frames: 60,
    },
    Case {
        file: "drift",
        preset: "Drift",
        frames: 60,
    },
];

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

/// The fixed frame every baseline is rendered under (a representative
/// mid-energy frame; Warp Drive is treble-reactive, so treble is up).
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
    for case in CASES {
        let fresh = renderer
            .capture_preset(case.preset, &frame, case.frames)
            .expect("capture preset");
        let path = golden_dir().join(format!("{}.png", case.file));

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
            "{:<8} mean {mean:.4} (tol {MEAN_TOL}) max_outlier {outlier} (tol {MAX_OUTLIER})",
            case.file
        );
        if mean > MEAN_TOL || outlier > MAX_OUTLIER {
            failures.push(format!(
                "{}: mean {mean:.4} / outlier {outlier} exceeds tolerance",
                case.file
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "golden drift beyond tolerance (bless with LMV_BLESS=1 if intended): {failures:#?}"
    );
}
