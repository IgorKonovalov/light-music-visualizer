//! Distinctness report (Plan 0013 Phase 4, ADVISORY — prints, never asserts).
//! Per family, capture every preset at one fixed frame and print two pairwise
//! matrices — pixel (`frame_diff`) and shape (`struct_diff`). Pairs whose shape
//! difference is below a small threshold are flagged as **near-duplicate
//! geometry**; the recolor case (high pixel diff, low shape diff) is the one to
//! catch. This tool only measures — redesigning too-similar presets is separate
//! content work (a Plan 0013 followup).
//!
//! Run with: `cargo test -p lmv-core --test distinctness -- --nocapture`

use lmv_core::dsp::AnalysisFrame;
use lmv_core::preset::{SystemKind, default_presets};
use lmv_core::render::{
    CaptureImage, HeadlessOptions, Renderer,
    metrics::{frame_diff, struct_diff},
};

const SIZE: u32 = 128;
const FRAMES: u32 = 60;
/// A `struct_diff` below this flags a pair as near-duplicate geometry.
const NEAR_DUP_STRUCT: f32 = 0.08;

fn headless() -> Renderer {
    Renderer::new_headless(HeadlessOptions {
        width: SIZE,
        height: SIZE,
        prefer_software: true,
    })
    .expect("headless renderer builds on the software adapter")
}

/// One representative non-silent frame, shared by every capture so the only
/// variable across a family is the preset.
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

fn print_matrix(
    title: &str,
    caps: &[(String, CaptureImage)],
    metric: impl Fn(&CaptureImage, &CaptureImage) -> f32,
) {
    println!("  {title}");
    print!("           ");
    for (name, _) in caps {
        print!("{:>8.8} ", name);
    }
    println!();
    for (rname, ra) in caps {
        print!("  {rname:>8.8} ");
        for (_, rb) in caps {
            print!("{:>8.3} ", metric(ra, rb));
        }
        println!();
    }
}

#[test]
fn report_family_distinctness() {
    let mut renderer = headless();
    let frame = fixed_frame();

    for (system, label) in [
        (SystemKind::FragmentField, "fragment_field"),
        (SystemKind::Swarm, "swarm"),
        (SystemKind::ParametricCurve, "parametric_curve"),
        (SystemKind::LSystem, "lsystem"),
        (SystemKind::StarPattern, "star_pattern"),
    ] {
        let names: Vec<String> = default_presets()
            .into_iter()
            .filter(|p| p.system == system)
            .map(|p| p.name)
            .collect();

        let caps: Vec<(String, CaptureImage)> = names
            .iter()
            .map(|name| {
                let img = renderer
                    .capture_preset(name, &frame, FRAMES)
                    .expect("capture preset");
                (name.clone(), img)
            })
            .collect();

        println!("\n=== family: {label} ({} presets) ===", caps.len());
        print_matrix("pixel (frame_diff)", &caps, frame_diff);
        print_matrix("shape (struct_diff)", &caps, struct_diff);

        let mut flagged = false;
        for i in 0..caps.len() {
            for j in (i + 1)..caps.len() {
                let sd = struct_diff(&caps[i].1, &caps[j].1);
                let pd = frame_diff(&caps[i].1, &caps[j].1);
                if sd < NEAR_DUP_STRUCT {
                    println!(
                        "  NEAR-DUP: {} ~ {}  (shape {sd:.3}, pixel {pd:.3})",
                        caps[i].0, caps[j].0
                    );
                    flagged = true;
                }
            }
        }
        if !flagged {
            println!("  (no near-duplicate geometry below shape {NEAR_DUP_STRUCT})");
        }
    }
}
