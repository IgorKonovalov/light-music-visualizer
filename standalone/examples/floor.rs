//! Bare-wgpu driver-floor spike (Plan 0012 Phase 2). NOT shipped — a Cargo
//! `examples/` target never links into the `lmv` binary, so it cannot grow the
//! release footprint it measures.
//!
//! Stands up ONLY the wgpu context — a winit window + `RenderContext::new` —
//! with no scenes, DSP, audio, or overlay, warms up briefly, then prints its
//! own working set + private commit. Subtracting this from the post-cull
//! standalone splits the fixed DX12 driver + wgpu + swapchain floor from our
//! per-system overhead (ADR-0010 point 3 / Plan 0012 Phase 2).
//!
//! Construct-only, not a clear-only pass: `RenderContext` exposes only `new`,
//! `resize`, and `surface_format` publicly — its device/queue/surface are
//! `pub(crate)` and this example compiles as a separate crate, so it cannot
//! encode its own render pass without growing core's public surface, which
//! Plan 0012 forbids for a diagnostic. That is a faithful floor because DX12
//! realizes the swapchain backbuffers at `surface.configure` (called inside
//! `new`), not lazily at first present — the fixed driver heap exists without
//! drawing a frame. It calls `RenderContext::new` verbatim, so the adapter /
//! device options match the real renderer's by construction (Plan 0012 risk).

use std::sync::Arc;
use std::time::{Duration, Instant};

use lmv_core::render::RenderContext;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

/// Warm up before sampling so the driver's lazy allocations settle into a
/// steady state comparable to the standalone's logged footprint.
const WARMUP: Duration = Duration::from_secs(3);

struct FloorProbe {
    /// Held only for its lifetime — the GPU context whose floor we measure. Its
    /// Drop frees the driver heap, so it must outlive the sample.
    _ctx: Option<RenderContext>,
    /// Held so the window (and the surface's Arc clone of it) stays alive.
    _window: Option<Arc<Window>>,
    /// When the context came up; the warmup clock is measured from here.
    started: Option<Instant>,
}

impl ApplicationHandler for FloorProbe {
    #[allow(
        clippy::disallowed_methods,
        reason = "shell-side warmup clock in a throwaway diagnostic; no core analysis here"
    )]
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self._ctx.is_some() {
            return;
        }
        // Same 1080p size the real app and the NFR 1 perf floor are quoted at.
        let attrs = Window::default_attributes()
            .with_title("lmv floor spike")
            .with_inner_size(winit::dpi::PhysicalSize::new(1920u32, 1080u32));
        let window = match event_loop.create_window(attrs) {
            Ok(window) => Arc::new(window),
            Err(err) => {
                eprintln!("failed to create window: {err}");
                event_loop.exit();
                return;
            }
        };
        let size = window.inner_size();
        match RenderContext::new(Arc::clone(&window), size.width, size.height) {
            Ok(ctx) => {
                self._ctx = Some(ctx);
                self._window = Some(window);
                self.started = Some(Instant::now());
            }
            Err(err) => {
                eprintln!("failed to create render context: {err}");
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        if let WindowEvent::CloseRequested = event {
            event_loop.exit();
        }
    }

    #[allow(
        clippy::disallowed_methods,
        reason = "shell-side warmup clock in a throwaway diagnostic; no core analysis here"
    )]
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(started) = self.started else {
            return;
        };
        // No rendering — just idle through the warmup, then sample and quit.
        if started.elapsed() < WARMUP {
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(100),
            ));
            return;
        }
        match current_memory() {
            Some((ws, private)) => println!(
                "floor: ws_bytes={ws} ws_mb={:.1} private_bytes={private} private_mb={:.1}",
                ws as f64 / (1024.0 * 1024.0),
                private as f64 / (1024.0 * 1024.0),
            ),
            None => eprintln!("floor: memory query failed"),
        }
        // Sample once: `exit()` isn't immediate, so disarm the timer to keep
        // `about_to_wait` from re-firing and printing again before we quit.
        self.started = None;
        event_loop.exit();
    }
}

/// Current working set + private commit (bytes), or `None` on query failure.
/// Inlined here (Plan 0012 Phase 2) rather than depending on the bin's `rss`
/// module, which reports working set only — the floor split wants both, and the
/// EX counters carry `PrivateUsage` (the private-commit figure ADR-0010 tracks).
#[cfg(windows)]
fn current_memory() -> Option<(u64, u64)> {
    use windows::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;

    let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
    let cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    // SAFETY: `counters` is a zeroed POD the call fills. The EX struct is a
    // layout-compatible superset of PROCESS_MEMORY_COUNTERS, so the cast plus
    // its own size is the documented way to receive PrivateUsage.
    // GetCurrentProcess returns a pseudo-handle that needs no close.
    let ok = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            std::ptr::from_mut(&mut counters).cast::<PROCESS_MEMORY_COUNTERS>(),
            cb,
        )
    };
    ok.is_ok()
        .then_some((counters.WorkingSetSize as u64, counters.PrivateUsage as u64))
}

/// This spike targets the Windows DX12 driver floor (Plan 0012); elsewhere the
/// number is meaningless, so report nothing rather than a misleading figure.
#[cfg(not(windows))]
fn current_memory() -> Option<(u64, u64)> {
    None
}

fn main() {
    // expect: init-time invariant — without an event loop there is no probe.
    let event_loop = EventLoop::new().expect("failed to create event loop");
    let mut probe = FloorProbe {
        _ctx: None,
        _window: None,
        started: None,
    };
    if let Err(err) = event_loop.run_app(&mut probe) {
        eprintln!("event loop error: {err}");
        std::process::exit(1);
    }
}
