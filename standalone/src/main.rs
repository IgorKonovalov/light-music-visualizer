#[cfg(target_os = "macos")]
mod capture_mac;
#[cfg(windows)]
mod capture_win;
mod diaglog;
mod overlay;
mod rss;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use diaglog::DiagLog;
use lmv_core::audio::{AudioFormat, SampleConsumer};
use lmv_core::dsp::Analyzer;
use lmv_core::render::{Renderer, TextRun};
use overlay::{OverlayAction, OverlayKey, OverlayState};
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

/// On-canvas active-preset-name label: top-left inset (device px), font size,
/// and a light near-white color legible over most scenes.
const NAME_INSET: f32 = 16.0;
const NAME_SIZE: f32 = 28.0;
const NAME_COLOR: [f32; 4] = [0.9, 0.95, 1.0, 1.0];

/// Browse-overlay list layout (device px) and row colors. The list starts below
/// the name label; each row is `ROW_H` tall; the highlighted row is brighter.
const LIST_INSET: f32 = 16.0;
const LIST_TOP: f32 = 64.0;
const ROW_H: f32 = 30.0;
const ROW_SIZE: f32 = 22.0;
const ROW_COLOR: [f32; 4] = [0.72, 0.78, 0.88, 0.95];
const ROW_HL_COLOR: [f32; 4] = [1.0, 0.88, 0.35, 1.0];

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
    /// Whether the diagnostics debug overlay is currently painted (toggled by F3).
    overlay_on: bool,
    /// The preset browse overlay's modal state (Tab toggles; Plan 0008).
    browse: OverlayState,
    /// ~1 Hz structured diagnostics logger (render thread only).
    diag_log: DiagLog,
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
            overlay_on: false,
            browse: OverlayState::new(),
            diag_log: DiagLog::new(resolve_log_path()),
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

        // Queue the on-canvas text for this frame (active name + browse list).
        self.queue_frame_text();

        if let Err(err) = self.renderer.render(&frame) {
            eprintln!("render error: {err}");
        }
        self.title_tick += 1;
        if self.title_tick >= TITLE_UPDATE_FRAMES {
            self.title_tick = 0;
            self.update_title();
        }
        // Structured 1 Hz log (render thread). RSS is queried lazily, only on the
        // seconds a sample is actually due.
        let metrics = self.renderer.metrics();
        self.diag_log.maybe_log(&metrics, rss::current_rss_bytes);
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

    /// Build this frame's on-canvas text and hand it to the renderer: always the
    /// active preset name in the corner, plus — while the browse overlay is open
    /// — the scrolled roster with the highlighted row distinct. Strings are
    /// owned locally so the renderer's `queue_text` (which copies them) needs no
    /// live borrow of the roster.
    fn queue_frame_text(&mut self) {
        let mut texts: Vec<String> = Vec::new();
        // (x, y, size, color) parallel to `texts`.
        let mut meta: Vec<(f32, f32, f32, [f32; 4])> = Vec::new();

        texts.push(self.renderer.preset_name().to_owned());
        meta.push((NAME_INSET, NAME_INSET, NAME_SIZE, NAME_COLOR));

        if self.browse.is_open() {
            let names: Vec<String> = self.renderer.preset_names().map(str::to_owned).collect();
            let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
            let visible = self.browse.visible(&name_refs);
            let highlight = self.browse.highlight();

            // A scroll window keeps the highlight on screen when the list is
            // taller than the canvas.
            let height = self.window.inner_size().height as f32;
            let max_rows = (((height - LIST_TOP) / ROW_H).floor() as usize).max(1);
            let scroll = highlight
                .saturating_sub(max_rows.saturating_sub(1))
                .min(visible.len().saturating_sub(max_rows));

            for (row, &(_abs, name)) in visible.iter().enumerate().skip(scroll).take(max_rows) {
                let y = LIST_TOP + (row - scroll) as f32 * ROW_H;
                let (marker, color) = if row == highlight {
                    ("> ", ROW_HL_COLOR)
                } else {
                    ("  ", ROW_COLOR)
                };
                texts.push(format!("{marker}{name}"));
                meta.push((LIST_INSET, y, ROW_SIZE, color));
            }
        }

        let runs: Vec<TextRun<'_>> = texts
            .iter()
            .zip(meta.iter())
            .map(|(t, &(x, y, size, color))| TextRun {
                text: t.as_str(),
                x,
                y,
                size,
                color,
            })
            .collect();
        self.renderer.queue_text(&runs);
    }

    /// Route a pressed key. Browse-overlay keys go through its state machine
    /// first (and are consumed while it is open); everything else falls through
    /// to the shell's own bindings — notably Space-cycle, which is suppressed
    /// while the overlay is open.
    fn handle_key(&mut self, code: KeyCode) {
        if let Some(key) = decode_overlay_key(code) {
            let names: Vec<String> = self.renderer.preset_names().map(str::to_owned).collect();
            let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
            match self.browse.handle_key(key, &name_refs) {
                OverlayAction::None => return, // closed + non-toggle: let it fall away
                OverlayAction::Redraw | OverlayAction::Close => {}
                OverlayAction::Select(index) => {
                    self.renderer.select_preset(index);
                    self.update_title();
                }
            }
            self.window.request_redraw();
            return;
        }

        match code {
            KeyCode::Space => {
                // Cycle only when the overlay is closed; open, keys are its own.
                if !self.browse.is_open() {
                    self.renderer.cycle_preset();
                    self.update_title();
                    self.window.request_redraw();
                }
            }
            KeyCode::F3 => {
                self.overlay_on = !self.overlay_on;
                self.renderer.set_overlay(self.overlay_on);
                self.window.request_redraw();
            }
            _ => {}
        }
    }
}

/// Map a physical key to the overlay's abstract key, or `None` for keys the
/// overlay does not own (which then reach the shell's own bindings).
fn decode_overlay_key(code: KeyCode) -> Option<OverlayKey> {
    Some(match code {
        KeyCode::Tab => OverlayKey::Toggle,
        KeyCode::ArrowUp => OverlayKey::Up,
        KeyCode::ArrowDown => OverlayKey::Down,
        KeyCode::Enter | KeyCode::NumpadEnter => OverlayKey::Enter,
        KeyCode::Escape => OverlayKey::Escape,
        _ => return None,
    })
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
                        physical_key: PhysicalKey::Code(code),
                        state: ElementState::Pressed,
                        repeat: false,
                        ..
                    },
                ..
            } => state.handle_key(code),
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

/// Resolve `diagnostics.log` under the per-user app dir (alongside the shared
/// `presets` dir). `None` if the OS data root can't be resolved — the logger
/// then silently no-ops (degrade, never crash — NFR 10).
fn resolve_log_path() -> Option<PathBuf> {
    preset_data_root().map(|root| root.join(APP_DIR_NAME).join("diagnostics.log"))
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
