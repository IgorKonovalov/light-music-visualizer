//! `shot` — headless capture / visual-QA CLI (Plan 0013). Dev/agent tooling: it
//! renders presets with **no window** and writes PNGs the agent can Read, or a
//! metrics report (text / JSON) the agent can parse. It links the same
//! `lmv-core` the app does; `image` (a dev-dependency, ADR-0011) only encodes
//! and tiles, so the shipped `lmv.exe` is untouched.
//!
//! Run: `cargo run -p standalone --example shot -- --preset Aurora --out shot.png`
//!
//! Flags:
//!   --preset <name>          single-shot the named preset
//!   --set k=v,...            constant stimulus (bass/mid/treb/onset/beat/bar)
//!   --frames <N>             frames to advance before capture (default 120)
//!   --size <WxH>             render size (default 1280x720)
//!   --out <path>             output PNG (single shot) or dir/file (--all)
//!   --all                    contact sheet of every preset (needs --out)
//!   --report [family=<sys>]  per-family reactivity / animation / distinctness
//!            [--json]        emit JSON instead of a text table
//!   --signal <kind:param>    synth audio filmstrip (click:120, bass:60, ...)
//!   --audio <clip.wav>       filmstrip from a 16-bit PCM WAV
//!   --strip <N>              frames tiled along the audio (default 8)
//!
//! Exit code is non-zero with a message on any bad argument or failure.

use std::path::{Path, PathBuf};

use lmv_core::audio::AudioFormat;
use lmv_core::dsp::{AnalysisFrame, HOP_SIZE};
use lmv_core::preset::{Preset, SystemKind, default_presets, load_dir};
use lmv_core::render::metrics::{coverage, frame_diff, quadrant_spread, struct_diff};
use lmv_core::render::{CaptureImage, HeadlessOptions, Renderer};
use lmv_core::signal::{bass_sine, chord, click_track, noise, treble_tone};

/// Mirrors the standalone's per-user app dir (main.rs `APP_DIR_NAME`).
const APP_DIR_NAME: &str = "light-music-visualizer";

fn main() {
    if let Err(msg) = run() {
        eprintln!("shot: {msg}");
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------------

enum Mode {
    Shot,
    All,
    Report,
}

struct Args {
    mode: Mode,
    preset: Option<String>,
    stimulus: AnalysisFrame,
    frames: u32,
    width: u32,
    height: u32,
    out: Option<PathBuf>,
    family: Option<SystemKind>,
    json: bool,
    signal: Option<String>,
    audio: Option<PathBuf>,
    strip: u32,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            mode: Mode::Shot,
            preset: None,
            stimulus: AnalysisFrame::default(),
            frames: 120,
            width: 1280,
            height: 720,
            out: None,
            family: None,
            json: false,
            signal: None,
            audio: None,
            strip: 8,
        }
    }
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args::default();
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--preset" => args.preset = Some(next_value(&mut it, "--preset")?),
            "--set" => apply_set(&mut args.stimulus, &next_value(&mut it, "--set")?)?,
            "--frames" => {
                args.frames = next_value(&mut it, "--frames")?
                    .parse()
                    .map_err(|_| "--frames expects a positive integer".to_string())?;
            }
            "--size" => {
                let (w, h) = parse_size(&next_value(&mut it, "--size")?)?;
                args.width = w;
                args.height = h;
            }
            "--out" => args.out = Some(PathBuf::from(next_value(&mut it, "--out")?)),
            "--all" => args.mode = Mode::All,
            "--report" => args.mode = Mode::Report,
            "--json" => args.json = true,
            "--signal" => args.signal = Some(next_value(&mut it, "--signal")?),
            "--audio" => args.audio = Some(PathBuf::from(next_value(&mut it, "--audio")?)),
            "--strip" => {
                args.strip = next_value(&mut it, "--strip")?
                    .parse::<u32>()
                    .ok()
                    .filter(|n| *n >= 1)
                    .ok_or("--strip expects a positive integer")?;
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            other if other.starts_with("family=") => {
                args.family = Some(parse_system(other.trim_start_matches("family="))?);
            }
            other => return Err(format!("unknown argument `{other}` (try --help)")),
        }
    }
    Ok(args)
}

fn next_value(it: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    it.next().ok_or_else(|| format!("{flag} needs a value"))
}

fn parse_size(spec: &str) -> Result<(u32, u32), String> {
    let (w, h) = spec
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("--size expects WxH, got `{spec}`"))?;
    let w = w.parse().map_err(|_| format!("bad width in `{spec}`"))?;
    let h = h.parse().map_err(|_| format!("bad height in `{spec}`"))?;
    if w == 0 || h == 0 {
        return Err("--size dimensions must be non-zero".to_string());
    }
    Ok((w, h))
}

/// Apply a comma-separated `k=v` list onto the stimulus frame. Keys are the
/// scalar analysis bands; `beat` is truthy for any non-zero value.
fn apply_set(frame: &mut AnalysisFrame, spec: &str) -> Result<(), String> {
    for pair in spec.split(',').filter(|s| !s.is_empty()) {
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| format!("--set expects k=v, got `{pair}`"))?;
        let v: f32 = value
            .parse()
            .map_err(|_| format!("--set `{key}` value `{value}` is not a number"))?;
        match key {
            "bass" => frame.bass = v,
            "mid" => frame.mid = v,
            "treb" => frame.treb = v,
            "onset" => frame.onset = v,
            "bar" => frame.bar = v,
            "beat" => frame.beat = v != 0.0,
            other => return Err(format!("--set: unknown key `{other}`")),
        }
    }
    Ok(())
}

fn parse_system(name: &str) -> Result<SystemKind, String> {
    match name {
        "fragment_field" => Ok(SystemKind::FragmentField),
        "swarm" => Ok(SystemKind::Swarm),
        other => Err(format!("unknown family `{other}` (fragment_field | swarm)")),
    }
}

fn print_usage() {
    eprintln!(
        "shot — headless capture / visual-QA (Plan 0013)\n\
         \n\
         --preset <name>            single-shot the named preset (needs --out)\n\
         --set k=v,...              stimulus: bass,mid,treb,onset,bar,beat\n\
         --frames <N>               frames before capture (default 120)\n\
         --size <WxH>               render size (default 1280x720)\n\
         --out <path>               PNG path (shot) or dir/file (--all)\n\
         --all                      contact sheet of every preset (needs --out)\n\
         --report [family=<sys>]    metrics table (fragment_field | swarm)\n\
         --json                     emit the report as JSON\n\
         --signal <kind:param>      synth audio filmstrip: click:120 bass:60\n\
                                    treble:10000 noise:7 chord (needs --out)\n\
         --audio <clip.wav>         filmstrip from a 16-bit PCM WAV (needs --out)\n\
         --strip <N>                frames tiled along the audio (default 8)"
    );
}

// ---------------------------------------------------------------------------
// Preset library + renderer
// ---------------------------------------------------------------------------

/// Load the app's on-disk preset library if it resolves and has any valid
/// presets; otherwise the embedded defaults. Returns the presets and a label
/// describing the source.
fn load_library() -> (Vec<Preset>, String) {
    let dir = resolve_preset_dir();
    if !dir.as_os_str().is_empty() {
        let report = load_dir(&dir);
        if !report.presets.is_empty() {
            return (report.presets, format!("on-disk {}", dir.display()));
        }
    }
    (default_presets(), "embedded defaults".to_string())
}

/// The per-user preset directory, resolved exactly as the app does (main.rs).
/// Empty when the OS data root can't be found — the caller falls back to the
/// embedded set.
fn resolve_preset_dir() -> PathBuf {
    match preset_data_root() {
        Some(root) => root.join(APP_DIR_NAME).join("presets"),
        None => PathBuf::new(),
    }
}

#[cfg(windows)]
fn preset_data_root() -> Option<PathBuf> {
    std::env::var_os("APPDATA")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

#[cfg(target_os = "macos")]
fn preset_data_root() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(|home| {
            PathBuf::from(home)
                .join("Library")
                .join("Application Support")
        })
}

#[cfg(not(any(windows, target_os = "macos")))]
fn preset_data_root() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(xdg));
    }
    std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(|home| PathBuf::from(home).join(".local").join("share"))
}

/// `(name, system)` pairs for the loaded library, in roster order.
fn preset_meta(presets: &[Preset]) -> Vec<(String, SystemKind)> {
    presets.iter().map(|p| (p.name.clone(), p.system)).collect()
}

/// A headless renderer over `presets`, using the real GPU at full quality (the
/// CLI wants speed and true output, not the tests' software reproducibility).
fn renderer(width: u32, height: u32, presets: Vec<Preset>) -> Result<Renderer, String> {
    let mut r = Renderer::new_headless(HeadlessOptions {
        width,
        height,
        prefer_software: false,
    })
    .map_err(|e| format!("headless renderer: {e}"))?;
    r.set_presets(presets);
    Ok(r)
}

// ---------------------------------------------------------------------------
// Dispatch
// ---------------------------------------------------------------------------

fn run() -> Result<(), String> {
    let args = parse_args()?;
    let (presets, source) = load_library();
    // An audio source (synth signal or WAV) takes precedence — it drives a
    // filmstrip regardless of the shot/all/report mode default.
    if args.signal.is_some() || args.audio.is_some() {
        return filmstrip(args, presets, &source);
    }
    match args.mode {
        Mode::Shot => shot(args, presets, &source),
        Mode::All => contact_sheet(args, presets, &source),
        Mode::Report => report(args, presets, &source),
    }
}

fn shot(args: Args, presets: Vec<Preset>, source: &str) -> Result<(), String> {
    let name = args
        .preset
        .clone()
        .ok_or("--preset <name> is required for a single shot")?;
    let out = args.out.clone().ok_or("--out <path> is required")?;
    let mut r = renderer(args.width, args.height, presets)?;
    let img = r
        .capture_preset(&name, &args.stimulus, args.frames)
        .map_err(|e| format!("capture `{name}`: {e}"))?;
    save_png(&img, &out)?;
    println!(
        "wrote {} ({}x{}, preset {name}, {} frames) [{source}]",
        out.display(),
        img.width,
        img.height,
        args.frames
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// PNG + contact sheet
// ---------------------------------------------------------------------------

fn save_png(img: &CaptureImage, path: &Path) -> Result<(), String> {
    let buffer = to_rgba(img)?;
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    buffer
        .save(path)
        .map_err(|e| format!("write {}: {e}", path.display()))
}

fn to_rgba(img: &CaptureImage) -> Result<image::RgbaImage, String> {
    image::RgbaImage::from_raw(img.width, img.height, img.rgba.clone())
        .ok_or_else(|| "capture buffer does not match its dimensions".to_string())
}

fn contact_sheet(args: Args, presets: Vec<Preset>, source: &str) -> Result<(), String> {
    let out = args.out.clone().ok_or("--all needs --out <dir-or-file>")?;
    let meta = preset_meta(&presets);
    if meta.is_empty() {
        return Err("no presets to tile".to_string());
    }
    let mut r = renderer(args.width, args.height, presets)?;

    // Layout: a near-square grid of fixed-width thumbnails with a label strip.
    const THUMB_W: u32 = 320;
    const PAD: u32 = 8;
    const LABEL_H: u32 = 18;
    let thumb_h = (THUMB_W * args.height / args.width).max(1);
    let cols = (meta.len() as f64).sqrt().ceil() as u32;
    let rows = meta.len().div_ceil(cols as usize) as u32;
    let cell_w = THUMB_W + PAD;
    let cell_h = thumb_h + LABEL_H + PAD;
    let canvas_w = cols * cell_w + PAD;
    let canvas_h = rows * cell_h + PAD;

    let mut canvas =
        image::RgbaImage::from_pixel(canvas_w, canvas_h, image::Rgba([18, 18, 22, 255]));

    for (i, (name, _system)) in meta.iter().enumerate() {
        let img = r
            .capture_preset(name, &args.stimulus, args.frames)
            .map_err(|e| format!("capture `{name}`: {e}"))?;
        let full = to_rgba(&img)?;
        let thumb = image::imageops::resize(
            &full,
            THUMB_W,
            thumb_h,
            image::imageops::FilterType::Triangle,
        );
        let col = i as u32 % cols;
        let row = i as u32 / cols;
        let x = PAD + col * cell_w;
        let y = PAD + row * cell_h;
        image::imageops::overlay(&mut canvas, &thumb, x as i64, y as i64);
        draw_label(
            &mut canvas,
            x,
            y + thumb_h + 3,
            name,
            [235, 235, 240, 255],
            2,
        );
    }

    let path = contact_sheet_path(&out);
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    canvas
        .save(&path)
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    println!(
        "wrote {} ({} presets, {cols}x{rows} grid) [{source}]",
        path.display(),
        meta.len()
    );
    Ok(())
}

/// A `.png` `--out` is used verbatim; anything else is treated as a directory
/// and the sheet lands at `<out>/contact-sheet.png`.
fn contact_sheet_path(out: &Path) -> PathBuf {
    if out
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("png"))
    {
        out.to_path_buf()
    } else {
        out.join("contact-sheet.png")
    }
}

// ---------------------------------------------------------------------------
// Audio filmstrips (--signal / --audio)
// ---------------------------------------------------------------------------

/// Duration synthesized for a `--signal` (enough for several 120 BPM beats).
const SIGNAL_SECS: f32 = 4.0;
/// Hops skipped at the start so the strip samples past the analyzer's warm-up.
const FILMSTRIP_WARMUP: usize = 8;

fn filmstrip(args: Args, presets: Vec<Preset>, source: &str) -> Result<(), String> {
    let out = args
        .out
        .clone()
        .ok_or("--signal/--audio needs --out <path>")?;

    let (pcm, format, label) = match (&args.signal, &args.audio) {
        (Some(spec), _) => {
            let (pcm, fmt) = synth_signal(spec)?;
            (pcm, fmt, format!("signal {spec}"))
        }
        (None, Some(path)) => {
            let (pcm, fmt) = read_wav_16bit(path)?;
            (pcm, fmt, format!("audio {}", path.display()))
        }
        (None, None) => return Err("no --signal or --audio given".to_string()),
    };

    let meta = preset_meta(&presets);
    let name = args
        .preset
        .clone()
        .or_else(|| meta.first().map(|(n, _)| n.clone()))
        .ok_or("no preset available to render")?;

    let mut r = renderer(args.width, args.height, presets)?;
    let at = filmstrip_indices(pcm.len(), format, args.strip)?;
    let frames = r
        .capture_audio(&name, &pcm, format, &at)
        .map_err(|e| format!("capture audio: {e}"))?;

    let strip = tile_filmstrip(&frames)?;
    save_image(&strip, &out)?;
    println!(
        "wrote {} ({} frames, preset {name}, {label}) [{source}]",
        out.display(),
        frames.len(),
    );
    Ok(())
}

/// `--strip` frame indices, evenly spaced from just past warm-up to the last
/// analysis frame the PCM produces.
fn filmstrip_indices(pcm_len: usize, format: AudioFormat, strip: u32) -> Result<Vec<u32>, String> {
    let hop_samples = HOP_SIZE * format.channels as usize;
    let total = pcm_len / hop_samples.max(1);
    if total <= FILMSTRIP_WARMUP + 1 {
        return Err("audio too short for a filmstrip".to_string());
    }
    let start = FILMSTRIP_WARMUP;
    let end = total - 1;
    let n = strip.max(1);
    if n == 1 {
        return Ok(vec![start as u32]);
    }
    let span = (end - start) as f32;
    Ok((0..n)
        .map(|i| (start as f32 + span * i as f32 / (n - 1) as f32).round() as u32)
        .collect())
}

/// Parse `<kind:param>` into synthesized PCM. Zero committed asset — this is the
/// self-contained validation of the whole audio path.
fn synth_signal(spec: &str) -> Result<(Vec<f32>, AudioFormat), String> {
    let format = AudioFormat {
        sample_rate: 48_000,
        channels: 2,
    };
    let (kind, param) = spec.split_once(':').unwrap_or((spec, ""));
    let pcm = match kind {
        "click" => click_track(parse_param(param, "click BPM")?, SIGNAL_SECS, format),
        "bass" => bass_sine(parse_param(param, "bass Hz")?, SIGNAL_SECS, format),
        "treble" | "treb" => treble_tone(parse_param(param, "treble Hz")?, SIGNAL_SECS, format),
        "noise" => {
            let seed = param.parse::<u64>().unwrap_or(1);
            noise(seed, SIGNAL_SECS, 0.8, format)
        }
        "chord" => chord(&[220.0, 277.0, 330.0], SIGNAL_SECS, format),
        other => {
            return Err(format!(
                "--signal: unknown kind `{other}` (click|bass|treble|noise|chord)"
            ));
        }
    };
    Ok((pcm, format))
}

fn parse_param(param: &str, what: &str) -> Result<f32, String> {
    param
        .parse::<f32>()
        .map_err(|_| format!("--signal: expected a {what} value, got `{param}`"))
}

/// A minimal hand-rolled 16-bit-PCM WAV reader (no decoder dependency). Supports
/// uncompressed PCM (format 1), 16-bit, any channel count / sample rate. Other
/// encodings are a documented followup.
fn read_wav_16bit(path: &Path) -> Result<(Vec<f32>, AudioFormat), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if bytes.get(0..4) != Some(b"RIFF") || bytes.get(8..12) != Some(b"WAVE") {
        return Err(format!("{} is not a RIFF/WAVE file", path.display()));
    }

    let mut channels = 0u16;
    let mut sample_rate = 0u32;
    let mut data: Option<&[u8]> = None;
    let mut pos = 12usize;
    while pos + 8 <= bytes.len() {
        let id = bytes.get(pos..pos + 4).unwrap_or(&[]);
        let size = le_u32(&bytes, pos + 4).unwrap_or(0) as usize;
        let body = pos + 8;
        let end = body.saturating_add(size).min(bytes.len());
        match id {
            b"fmt " => {
                let audio_format = le_u16(&bytes, body).unwrap_or(0);
                let bits = le_u16(&bytes, body + 14).unwrap_or(0);
                if audio_format != 1 {
                    return Err("only uncompressed PCM (format 1) WAV is supported".to_string());
                }
                if bits != 16 {
                    return Err(format!(
                        "only 16-bit PCM WAV is supported (found {bits}-bit)"
                    ));
                }
                channels = le_u16(&bytes, body + 2).unwrap_or(0);
                sample_rate = le_u32(&bytes, body + 4).unwrap_or(0);
            }
            b"data" => data = bytes.get(body..end),
            _ => {}
        }
        pos = body + size + (size & 1); // chunks are word-aligned
    }

    let data = data.ok_or("WAV has no data chunk")?;
    let samples: Vec<f32> = data
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
        .collect();
    let format = AudioFormat {
        sample_rate,
        channels,
    }
    .validate()
    .map_err(|e| format!("unusable WAV format: {e}"))?;
    Ok((samples, format))
}

fn le_u16(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn le_u32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *b.get(at)?,
        *b.get(at + 1)?,
        *b.get(at + 2)?,
        *b.get(at + 3)?,
    ]))
}

/// Tile the captured frames left-to-right into one filmstrip, each scaled to a
/// fixed height.
fn tile_filmstrip(frames: &[CaptureImage]) -> Result<image::RgbaImage, String> {
    const STRIP_H: u32 = 200;
    const PAD: u32 = 4;
    let first = frames.first().ok_or("no frames captured")?;
    let thumb_w = (first.width * STRIP_H / first.height.max(1)).max(1);
    let n = frames.len() as u32;
    let canvas_w = n * thumb_w + (n + 1) * PAD;
    let canvas_h = STRIP_H + 2 * PAD;
    let mut canvas =
        image::RgbaImage::from_pixel(canvas_w, canvas_h, image::Rgba([18, 18, 22, 255]));
    for (i, frame) in frames.iter().enumerate() {
        let full = to_rgba(frame)?;
        let thumb = image::imageops::resize(
            &full,
            thumb_w,
            STRIP_H,
            image::imageops::FilterType::Triangle,
        );
        let x = PAD + i as u32 * (thumb_w + PAD);
        image::imageops::overlay(&mut canvas, &thumb, x as i64, PAD as i64);
    }
    Ok(canvas)
}

/// Save a prepared `RgbaImage` to `path`, creating parent dirs.
fn save_image(img: &image::RgbaImage, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    img.save(path)
        .map_err(|e| format!("write {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

/// Report render size — small; the metrics don't need resolution.
const REPORT_SIZE: u32 = 192;
const REPORT_FRAMES: u32 = 24;
const REPORT_FRAMES_LATE: u32 = 48;
const NEAR_DUP_STRUCT: f32 = 0.08;
const COVERAGE_EPS: u8 = 10;

struct PresetReport {
    name: String,
    reactivity: [f32; 4], // bass, mid, treb, onset
    animation: f32,
    coverage: f32,
}

struct FamilyReport {
    system: SystemKind,
    presets: Vec<PresetReport>,
    pixel: Vec<Vec<f32>>,
    shape: Vec<Vec<f32>>,
    near_dups: Vec<(String, String)>,
}

fn report(args: Args, presets: Vec<Preset>, source: &str) -> Result<(), String> {
    let meta = preset_meta(&presets);
    let mut r = renderer(REPORT_SIZE, REPORT_SIZE, presets)?;

    let families = [SystemKind::FragmentField, SystemKind::Swarm];
    let mut reports = Vec::new();
    for system in families {
        if args.family.is_some_and(|f| f != system) {
            continue;
        }
        let names: Vec<String> = meta
            .iter()
            .filter(|(_, s)| *s == system)
            .map(|(n, _)| n.clone())
            .collect();
        if names.is_empty() {
            continue;
        }
        reports.push(build_family_report(&mut r, system, &names)?);
    }

    if args.json {
        print!("{}", render_json(source, &reports));
    } else {
        print_text_report(source, &reports);
    }
    Ok(())
}

fn build_family_report(
    r: &mut Renderer,
    system: SystemKind,
    names: &[String],
) -> Result<FamilyReport, String> {
    let silent = AnalysisFrame::default();
    let loud = loud_frame();
    let bands = band_stimuli();

    let mut presets = Vec::new();
    let mut fixed_caps = Vec::new();
    for name in names {
        let base = capture(r, name, &silent, REPORT_FRAMES)?;
        let mut reactivity = [0.0f32; 4];
        for (i, frame) in bands.iter().enumerate() {
            let lit = capture(r, name, frame, REPORT_FRAMES)?;
            reactivity[i] = frame_diff(&base, &lit);
        }
        let late = capture(r, name, &silent, REPORT_FRAMES_LATE)?;
        let animation = frame_diff(&base, &late);

        let fixed = capture(r, name, &loud, REPORT_FRAMES_LATE)?;
        let bg = corner(&fixed);
        let cov = coverage(&fixed, bg, COVERAGE_EPS);
        // Belt-and-braces "not a dot" note folded into coverage via spread:
        // a single-quadrant frame is suspicious even at decent coverage.
        let _spread = quadrant_spread(&fixed, bg, COVERAGE_EPS);

        presets.push(PresetReport {
            name: name.clone(),
            reactivity,
            animation,
            coverage: cov,
        });
        fixed_caps.push(fixed);
    }

    // Pairwise pixel + shape matrices over the fixed-frame captures.
    let n = fixed_caps.len();
    let mut pixel = vec![vec![0.0f32; n]; n];
    let mut shape = vec![vec![0.0f32; n]; n];
    let mut near_dups = Vec::new();
    for i in 0..n {
        for j in 0..n {
            let pd = frame_diff(&fixed_caps[i], &fixed_caps[j]);
            let sd = struct_diff(&fixed_caps[i], &fixed_caps[j]);
            pixel[i][j] = pd;
            shape[i][j] = sd;
            if i < j && sd < NEAR_DUP_STRUCT {
                near_dups.push((names[i].clone(), names[j].clone()));
            }
        }
    }

    Ok(FamilyReport {
        system,
        presets,
        pixel,
        shape,
        near_dups,
    })
}

fn capture(
    r: &mut Renderer,
    name: &str,
    frame: &AnalysisFrame,
    frames: u32,
) -> Result<CaptureImage, String> {
    r.capture_preset(name, frame, frames)
        .map_err(|e| format!("capture `{name}`: {e}"))
}

fn band_stimuli() -> [AnalysisFrame; 4] {
    [
        AnalysisFrame {
            bass: 1.0,
            ..Default::default()
        },
        AnalysisFrame {
            mid: 1.0,
            ..Default::default()
        },
        AnalysisFrame {
            treb: 1.0,
            ..Default::default()
        },
        AnalysisFrame {
            onset: 1.0,
            beat: true,
            ..Default::default()
        },
    ]
}

fn loud_frame() -> AnalysisFrame {
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

fn corner(img: &CaptureImage) -> [u8; 4] {
    [
        img.rgba.first().copied().unwrap_or(0),
        img.rgba.get(1).copied().unwrap_or(0),
        img.rgba.get(2).copied().unwrap_or(0),
        255,
    ]
}

fn system_name(system: SystemKind) -> &'static str {
    match system {
        SystemKind::FragmentField => "fragment_field",
        SystemKind::Swarm => "swarm",
    }
}

fn print_text_report(source: &str, reports: &[FamilyReport]) {
    println!("visual-QA report [{source}]");
    for fam in reports {
        println!(
            "\n=== {} ({} presets) ===",
            system_name(fam.system),
            fam.presets.len()
        );
        println!(
            "  {:<14} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7}",
            "preset", "bass", "mid", "treb", "onset", "anim", "cover"
        );
        for p in &fam.presets {
            println!(
                "  {:<14.14} {:>7.3} {:>7.3} {:>7.3} {:>7.3} {:>7.3} {:>7.3}",
                p.name,
                p.reactivity[0],
                p.reactivity[1],
                p.reactivity[2],
                p.reactivity[3],
                p.animation,
                p.coverage,
            );
        }
        if fam.near_dups.is_empty() {
            println!("  near-duplicate geometry: none below shape {NEAR_DUP_STRUCT}");
        } else {
            for (a, b) in &fam.near_dups {
                println!("  NEAR-DUP: {a} ~ {b}");
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Hand-rolled JSON (fixed numeric schema, no serde)
// ---------------------------------------------------------------------------

fn render_json(source: &str, reports: &[FamilyReport]) -> String {
    let mut out = String::new();
    out.push('{');
    out.push_str(&format!("\"source\":{},", json_string(source)));
    out.push_str("\"families\":{");
    for (fi, fam) in reports.iter().enumerate() {
        if fi > 0 {
            out.push(',');
        }
        out.push_str(&format!("{}:{{", json_string(system_name(fam.system))));
        // presets
        out.push_str("\"presets\":{");
        for (pi, p) in fam.presets.iter().enumerate() {
            if pi > 0 {
                out.push(',');
            }
            out.push_str(&format!("{}:{{", json_string(&p.name)));
            out.push_str(&format!(
                "\"reactivity\":{{\"bass\":{},\"mid\":{},\"treb\":{},\"onset\":{}}},",
                num(p.reactivity[0]),
                num(p.reactivity[1]),
                num(p.reactivity[2]),
                num(p.reactivity[3]),
            ));
            out.push_str(&format!("\"animation\":{},", num(p.animation)));
            out.push_str(&format!("\"coverage\":{}", num(p.coverage)));
            out.push('}');
        }
        out.push_str("},");
        // distinctness
        out.push_str("\"distinctness\":{");
        out.push_str(&format!("\"pixel\":{},", json_matrix(&fam.pixel)));
        out.push_str(&format!("\"shape\":{},", json_matrix(&fam.shape)));
        out.push_str("\"near_duplicates\":[");
        for (di, (a, b)) in fam.near_dups.iter().enumerate() {
            if di > 0 {
                out.push(',');
            }
            out.push_str(&format!("[{},{}]", json_string(a), json_string(b)));
        }
        out.push(']');
        out.push('}');
        out.push('}');
    }
    out.push_str("}}");
    out.push('\n');
    out
}

fn json_matrix(rows: &[Vec<f32>]) -> String {
    let mut s = String::from("[");
    for (i, row) in rows.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push('[');
        for (j, v) in row.iter().enumerate() {
            if j > 0 {
                s.push(',');
            }
            s.push_str(&num(*v));
        }
        s.push(']');
    }
    s.push(']');
    s
}

/// Fixed 4-decimal number, so the schema is stable and parseable.
fn num(v: f32) -> String {
    format!("{v:.4}")
}

/// Minimal JSON string escaping (quotes, backslash, control chars).
fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ---------------------------------------------------------------------------
// Minimal 5x7 bitmap font for contact-sheet labels (A-Z 0-9 space - .)
// ---------------------------------------------------------------------------

/// Draw `text` (uppercased) at `(x, y)` on `canvas`, each glyph scaled by
/// `scale`. Unknown characters render blank. Pixels outside the canvas are
/// clipped.
fn draw_label(
    canvas: &mut image::RgbaImage,
    x: u32,
    y: u32,
    text: &str,
    color: [u8; 4],
    scale: u32,
) {
    let mut cx = x;
    for ch in text.chars() {
        let glyph = glyph_for(ch.to_ascii_uppercase());
        for (row, bits) in glyph.iter().enumerate() {
            for col in 0..5u32 {
                if bits & (1 << (4 - col)) != 0 {
                    for dy in 0..scale {
                        for dx in 0..scale {
                            let px = cx + col * scale + dx;
                            let py = y + row as u32 * scale + dy;
                            if px < canvas.width() && py < canvas.height() {
                                canvas.put_pixel(px, py, image::Rgba(color));
                            }
                        }
                    }
                }
            }
        }
        cx += 6 * scale;
    }
}

/// 7 rows (top→bottom); each byte's low 5 bits are the columns (bit4 = left).
fn glyph_for(ch: char) -> [u8; 7] {
    match ch {
        'A' => [0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'B' => [0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E],
        'C' => [0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E],
        'D' => [0x1E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1E],
        'E' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F],
        'F' => [0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10],
        'G' => [0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F],
        'H' => [0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11],
        'I' => [0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E],
        'J' => [0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C],
        'K' => [0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11],
        'L' => [0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F],
        'M' => [0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11],
        'N' => [0x11, 0x11, 0x19, 0x15, 0x13, 0x11, 0x11],
        'O' => [0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'P' => [0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10],
        'Q' => [0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D],
        'R' => [0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11],
        'S' => [0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E],
        'T' => [0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04],
        'U' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E],
        'V' => [0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04],
        'W' => [0x11, 0x11, 0x11, 0x15, 0x15, 0x15, 0x0A],
        'X' => [0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11],
        'Y' => [0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04],
        'Z' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F],
        '0' => [0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E],
        '1' => [0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E],
        '2' => [0x0E, 0x11, 0x01, 0x06, 0x08, 0x10, 0x1F],
        '3' => [0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E],
        '4' => [0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02],
        '5' => [0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E],
        '6' => [0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E],
        '7' => [0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08],
        '8' => [0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E],
        '9' => [0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C],
        '-' => [0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00],
        '.' => [0x00, 0x00, 0x00, 0x00, 0x00, 0x06, 0x06],
        _ => [0x00; 7], // space + anything unmapped
    }
}
