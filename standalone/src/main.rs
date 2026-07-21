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

/// Directory the standalone loads presets from and watches for hot-reload,
/// resolved relative to the working directory (the repo root under `cargo run`).
/// If it is missing or empty the renderer keeps its embedded default presets.
const PRESET_DIR: &str = "presets";
/// How often to re-scan the preset directory for edits.
const PRESET_POLL: Duration = Duration::from_millis(500);

struct AppState {
    window: Arc<Window>,
    renderer: Renderer,
    analyzer: Analyzer,
    consumer: Option<SampleConsumer>,
    // Held for its Drop: stops the capture thread with the app.
    _capture: Option<capture_handle::Handle>,
    scratch: Vec<f32>,
    occluded: bool,
    fps_window_start: Instant,
    fps_frames: u32,
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

        // Load presets from disk over the renderer's embedded defaults, and
        // record the directory signature so later edits hot-reload.
        let preset_dir = PathBuf::from(PRESET_DIR);
        reload_presets(&mut renderer, &preset_dir);
        let preset_sig = dir_signature(&preset_dir);

        let (capture, consumer, format) = start_capture();
        let analyzer = Analyzer::new(format)
            .expect("capture layer already validated this format at the boundary");

        // Frame pacing is a shell concern; the core stays clock-free (determinism).
        #[allow(
            clippy::disallowed_methods,
            reason = "FPS-window / poll start; wall-clock pacing lives in the shell, not core analysis"
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
            fps_window_start: start,
            fps_frames: 0,
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
        self.count_frame();
        self.window.request_redraw();
    }

    #[allow(
        clippy::disallowed_methods,
        reason = "FPS accounting reads the wall clock; core analysis stays clock-free"
    )]
    fn count_frame(&mut self) {
        self.fps_frames += 1;
        let elapsed = self.fps_window_start.elapsed();
        if elapsed >= Duration::from_secs(1) {
            let fps = self.fps_frames as f32 / elapsed.as_secs_f32();
            let preset = self.renderer.preset_name();
            let system = self.renderer.active_system_name();
            self.window
                .set_title(&format!("{APP_TITLE} — {preset} [{system}] — {fps:.0} fps"));
            self.fps_window_start = Instant::now();
            self.fps_frames = 0;
        }
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
                let preset = state.renderer.cycle_preset().to_owned();
                let system = state.renderer.active_system_name();
                state
                    .window
                    .set_title(&format!("{APP_TITLE} — {preset} [{system}]"));
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
