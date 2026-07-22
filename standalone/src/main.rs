#[cfg(target_os = "macos")]
mod capture_mac;
#[cfg(windows)]
mod capture_win;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use lmv_core::audio::{AudioFormat, SampleConsumer};
use lmv_core::dsp::Analyzer;
use lmv_core::render::Renderer;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

/// How often the render loop wakes to keep DSP fed while hidden (NFR 1:
/// near-zero GPU in the background, analysis stays warm).
const HIDDEN_TICK: Duration = Duration::from_millis(100);

/// Window-title prefix: app name plus the application version. `CARGO_PKG_VERSION`
/// resolves at compile time to the single [workspace.package].version (ADR-0005).
const APP_TITLE: &str = concat!("light-music-visualizer ", env!("CARGO_PKG_VERSION"));

/// Per-user application directory name, used under the OS data root to build
/// the shared preset directory (the foobar plugin resolves the same path).
const APP_DIR_NAME: &str = "light-music-visualizer";
/// How often to re-scan the preset directory for edits.
const PRESET_POLL: Duration = Duration::from_millis(500);
/// Refresh the window title (fps + p99) every this many rendered frames — a
/// frame-count cadence keeps the shell clock-free for the title; the numbers
/// themselves come from the core's diagnostics.
const TITLE_UPDATE_FRAMES: u32 = 30;

struct AppState {
    window: Arc<Window>,
    renderer: Renderer,
    analyzer: Analyzer,
    consumer: Option<SampleConsumer>,
    // Held for its Drop: stops the capture thread with the app.
    _capture: Option<capture_handle::Handle>,
    scratch: Vec<f32>,
    occluded: bool,
    /// Frames since the last title refresh (title shows core-sourced fps + p99).
    title_tick: u32,
    /// Preset directory watched for hot-reload, with its last-seen signature
    /// and poll deadline.
    preset_dir: PathBuf,
    preset_sig: Option<(u128, usize)>,
    last_preset_poll: Instant,
}

/// Narrow alias so the non-Windows build (no capture until Phase 9) compiles
/// the same struct shape.
mod capture_handle {
    #[cfg(target_os = "macos")]
    pub type Handle = crate::capture_mac::CaptureHandle;
    #[cfg(windows)]
    pub type Handle = crate::capture_win::CaptureHandle;
    #[cfg(not(any(windows, target_os = "macos")))]
    pub type Handle = ();
}

impl AppState {
    fn new(window: Arc<Window>) -> Self {
        let size = window.inner_size();
        let mut renderer = Renderer::new(Arc::clone(&window), size.width, size.height)
            .unwrap_or_else(|err| {
                eprintln!("renderer init failed: {err}");
                std::process::exit(1);
            });

        // Resolve the per-user preset directory, seed the curated set into it
        // on first run (write-if-absent), then load it over the renderer's
        // embedded defaults and record the signature so later edits hot-reload.
        // Any failure degrades to the embedded defaults (NFR 10).
        let preset_dir = resolve_preset_dir();
        seed_preset_dir(&preset_dir);
        reload_presets(&mut renderer, &preset_dir);
        let preset_sig = dir_signature(&preset_dir);

        // Collect rolling frame-time stats from the first frame so the title
        // shows live fps/p99 (the overlay itself stays off until F3 — Plan 0011).
        renderer.enable_diagnostics(true);

        let (capture, consumer, format) = start_capture();
        let analyzer = Analyzer::new(format)
            .expect("capture layer already validated this format at the boundary");

        // Frame pacing is a shell concern; the core stays clock-free (determinism).
        #[allow(
            clippy::disallowed_methods,
            reason = "preset-poll start; wall-clock pacing lives in the shell, not core analysis"
        )]
        let start = Instant::now();
        Self {
            window,
            renderer,
            analyzer,
            consumer,
            _capture: capture,
            scratch: vec![0.0; 32_768],
            occluded: false,
            title_tick: 0,
            preset_dir,
            preset_sig,
            last_preset_poll: start,
        }
    }

    /// Re-scan the preset directory if the poll interval has elapsed and its
    /// signature changed, hot-reloading on any edit. Keeps the current set if
    /// the reload yields nothing valid (degrade, never crash — NFR 10).
    #[allow(
        clippy::disallowed_methods,
        reason = "preset-poll pacing reads the wall clock; core analysis stays clock-free"
    )]
    fn poll_presets(&mut self) {
        if self.last_preset_poll.elapsed() < PRESET_POLL {
            return;
        }
        self.last_preset_poll = Instant::now();
        let sig = dir_signature(&self.preset_dir);
        if sig == self.preset_sig {
            return;
        }
        self.preset_sig = sig;
        reload_presets(&mut self.renderer, &self.preset_dir);
    }

    /// Drain whatever audio arrived since last frame into the analyzer.
    /// Runs even while hidden so visuals resume in sync.
    fn pump_audio(&mut self) {
        if let Some(consumer) = self.consumer.as_mut() {
            loop {
                let n = consumer.pop_samples(&mut self.scratch);
                if n == 0 {
                    break;
                }
                self.analyzer.push_interleaved(&self.scratch[..n]);
            }
        }
    }

    fn hidden(&self) -> bool {
        let size = self.window.inner_size();
        self.occluded || size.width == 0 || size.height == 0
    }

    fn redraw(&mut self) {
        self.pump_audio();
        if self.hidden() {
            return;
        }
        self.poll_presets();
        let frame = self.analyzer.take_frame();
        if let Err(err) = self.renderer.render(&frame) {
            eprintln!("render error: {err}");
        }
        self.title_tick += 1;
        if self.title_tick >= TITLE_UPDATE_FRAMES {
            self.title_tick = 0;
            self.update_title();
        }
        self.window.request_redraw();
    }

    /// Refresh the window title with the preset, system, and the core's
    /// diagnostics (fps + p99). No wall-clock read — the numbers come from the
    /// core's gated clock, the cadence from a frame counter.
    fn update_title(&mut self) {
        let m = self.renderer.metrics();
        let preset = self.renderer.preset_name();
        let system = self.renderer.active_system_name();
        self.window.set_title(&format!(
            "{APP_TITLE} — {preset} [{system}] — {:.0} fps  p99 {:.1} ms",
            m.fps, m.frame_ms_p99
        ));
    }
}

#[cfg(windows)]
fn start_capture() -> (
    Option<capture_handle::Handle>,
    Option<SampleConsumer>,
    AudioFormat,
) {
    match capture_win::start() {
        Ok((handle, consumer)) => {
            let format = handle.format();
            (Some(handle), Some(consumer), format)
        }
        Err(err) => {
            eprintln!("loopback capture unavailable ({err}); rendering without audio");
            (
                None,
                None,
                AudioFormat {
                    sample_rate: 48_000,
                    channels: 2,
                },
            )
        }
    }
}

#[cfg(target_os = "macos")]
fn start_capture() -> (
    Option<capture_handle::Handle>,
    Option<SampleConsumer>,
    AudioFormat,
) {
    match capture_mac::start() {
        Ok((handle, consumer)) => {
            let format = handle.format();
            (Some(handle), Some(consumer), format)
        }
        Err(err) => {
            eprintln!("ScreenCaptureKit capture unavailable ({err}); rendering without audio");
            (
                None,
                None,
                AudioFormat {
                    sample_rate: 48_000,
                    channels: 2,
                },
            )
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn start_capture() -> (
    Option<capture_handle::Handle>,
    Option<SampleConsumer>,
    AudioFormat,
) {
    // No capture path on this platform; render silence-driven visuals.
    (
        None,
        None,
        AudioFormat {
            sample_rate: 48_000,
            channels: 2,
        },
    )
}

struct App {
    state: Option<AppState>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.is_none() {
            // 1080p default: the size the NFR 1 performance floor is quoted at.
            let attrs = Window::default_attributes()
                .with_title(APP_TITLE)
                .with_inner_size(winit::dpi::PhysicalSize::new(1920u32, 1080u32));
            match event_loop.create_window(attrs) {
                Ok(window) => {
                    let state = AppState::new(Arc::new(window));
                    state.window.request_redraw();
                    self.state = Some(state);
                }
                Err(err) => {
                    eprintln!("failed to create window: {err}");
                    event_loop.exit();
                }
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(state) = self.state.as_mut() else {
            if let WindowEvent::CloseRequested = event {
                event_loop.exit();
            }
            return;
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                state.renderer.resize(size.width, size.height);
                state.window.request_redraw();
            }
            WindowEvent::Occluded(occluded) => {
                state.occluded = occluded;
                if !occluded {
                    state.window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => state.redraw(),
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(KeyCode::Space),
                        state: ElementState::Pressed,
                        repeat: false,
                        ..
                    },
                ..
            } => {
                state.renderer.cycle_preset();
                state.update_title();
                state.window.request_redraw();
            }
            _ => {}
        }
    }

    #[allow(
        clippy::disallowed_methods,
        reason = "hidden-window wake deadline; shell frame pacing, not core analysis"
    )]
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(state) = self.state.as_mut() else {
            return;
        };
        if state.hidden() {
            // Hidden: no redraws (near-zero GPU), but wake periodically to
            // keep draining audio so the picture is current on return.
            state.pump_audio();
            event_loop.set_control_flow(ControlFlow::WaitUntil(Instant::now() + HIDDEN_TICK));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

/// Resolve the shared per-user preset directory, hand-rolled per-OS so we add
/// no runtime dependency (NFR 4). Windows: `%APPDATA%\light-music-visualizer\
/// presets`. macOS: `~/Library/Application Support/light-music-visualizer/
/// presets`. Other: `$XDG_DATA_HOME` (or `~/.local/share`) plus the same
/// suffix. Returns an empty path if the OS data root can't be resolved, so the
/// caller keeps the renderer's embedded defaults (degrade, never crash — NFR 10).
fn resolve_preset_dir() -> PathBuf {
    match preset_data_root() {
        Some(root) => root.join(APP_DIR_NAME).join("presets"),
        None => {
            eprintln!("could not resolve a per-user data directory; keeping embedded presets");
            PathBuf::new()
        }
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

/// Seed the embedded curated set into `dir` on first run. An unresolved
/// (empty) path or a seeding error is logged and otherwise ignored — the
/// renderer's embedded defaults remain (degrade, never crash — NFR 10).
fn seed_preset_dir(dir: &Path) {
    if dir.as_os_str().is_empty() {
        return;
    }
    match lmv_core::preset::seed_dir(dir) {
        Ok(0) => {}
        Ok(n) => eprintln!("seeded {n} curated preset(s) into {}", dir.display()),
        Err(err) => eprintln!("could not seed presets into {}: {err}", dir.display()),
    }
}

/// A cheap change signature for the preset directory: the newest `.toml` mtime
/// (nanoseconds) and the file count. Any edit bumps an mtime; add/remove
/// changes the count. `None` if the directory can't be read.
fn dir_signature(dir: &Path) -> Option<(u128, usize)> {
    let mut latest = 0u128;
    let mut count = 0usize;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "toml") {
            count += 1;
            if let Ok(modified) = entry.metadata().and_then(|m| m.modified())
                && let Ok(since) = modified.duration_since(std::time::UNIX_EPOCH)
            {
                latest = latest.max(since.as_nanos());
            }
        }
    }
    Some((latest, count))
}

/// Load presets from `dir` and, if any compiled, install them on the renderer.
/// Malformed files are reported to stderr; a directory with no valid presets
/// leaves the renderer's current set (embedded defaults or last good) in place.
fn reload_presets(renderer: &mut Renderer, dir: &Path) {
    let report = lmv_core::preset::load_dir(dir);
    for (path, err) in &report.errors {
        eprintln!("preset {}: {err}", path.display());
    }
    if report.presets.is_empty() {
        if !report.errors.is_empty() {
            eprintln!("no valid presets in {}; keeping current set", dir.display());
        }
    } else {
        eprintln!(
            "loaded {} preset(s) from {}",
            report.presets.len(),
            dir.display()
        );
        renderer.set_presets(report.presets);
    }
}

fn main() {
    // expect: init-time invariant — without an event loop there is no app.
    let event_loop = EventLoop::new().expect("failed to create event loop");
    let mut app = App { state: None };
    if let Err(err) = event_loop.run_app(&mut app) {
        eprintln!("event loop error: {err}");
        std::process::exit(1);
    }
}
