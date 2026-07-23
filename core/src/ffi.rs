//! The versioned C ABI — the single FFI seam of the project (ADR-0001).
//!
//! **This surface is a contract.** The C++ foobar2000 shim compiles against
//! `core/include/lmv_core.h` separately from this crate, so any change to the
//! shape of these functions is an ADR-worthy event, not a casual edit. Keep
//! it minimal: create/free, push samples, attach window, render, resize,
//! cycle scene, load presets, version query.
//!
//! # Threading contract (mirrored in the header)
//! - `lmv_push_samples` is called from at most one thread at a time (the
//!   host's audio/visualisation thread). It is real-time safe: lock-free,
//!   no allocation.
//! - Every other function is called from at most one thread at a time (the
//!   host's UI/render thread), never concurrently with `lmv_create`/
//!   `lmv_free` on the same handle.
//! - The two thread roles may run concurrently; they meet only at the
//!   lock-free ring inside.
//!
//! Panics never cross the boundary: every entry point catches unwinds and
//! maps them to `LMV_ERR_PANIC`.

// Hot-path panic-denial pragma (Plan 0002 Phase 2). The FFI seam must not
// panic; unwinds are caught explicitly and mapped to error codes.
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::unreachable
)]

use std::cell::UnsafeCell;
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::audio::{AudioFormat, SampleConsumer, SampleProducer, intake};
use crate::dsp::Analyzer;
use crate::preset::{self, Preset};
use crate::render::Renderer;

/// Bump on any ABI shape change (with the accompanying ADR). v2 added
/// `lmv_load_presets` (ADR-0006); v3 added `lmv_set_debug` + `lmv_get_metrics`
/// and the `LmvMetrics` struct (ADR-0008).
pub const LMV_ABI_VERSION: u32 = 3;

/// Call succeeded.
pub const LMV_OK: i32 = 0;
/// A null handle or otherwise invalid argument was passed.
pub const LMV_ERR_INVALID_ARG: i32 = -1;
/// The stream format was rejected at the boundary.
pub const LMV_ERR_FORMAT: i32 = -2;
/// Rendering failed (surface/device error).
pub const LMV_ERR_RENDER: i32 = -3;
/// A render call was made before a window was attached.
pub const LMV_ERR_NO_WINDOW: i32 = -4;
/// A Rust panic was caught at the boundary and mapped to an error.
pub const LMV_ERR_PANIC: i32 = -5;
/// The operation is unsupported on this platform.
pub const LMV_ERR_UNSUPPORTED: i32 = -6;

/// Debug flags (ADR-0008). No flags — a clean scene, metrics still collected.
pub const LMV_DEBUG_OFF: u32 = 0;
/// Draw the on-screen diagnostics overlay. Higher bits are reserved, ignored.
pub const LMV_DEBUG_OVERLAY: u32 = 1 << 0;

/// Environment variable read once at [`lmv_create`] to seed the default debug
/// flags (a boundary read). Any of `1`/`true`/`on`/`yes` (case-insensitive)
/// turns the overlay on at boot; a host can still flip it live via
/// [`lmv_set_debug`].
const DEBUG_OVERLAY_ENV: &str = "LMV_DEBUG_OVERLAY";

/// The diagnostics snapshot the host reads over the ABI (ADR-0008). Plain data,
/// caller-allocated — no allocation crosses the boundary. Leads with
/// `struct_size` + `abi_version` so later fields append without a v4 bump
/// (forward-extensible by size). Layout mirrors `core/include/lmv_core.h`;
/// process RSS is deliberately NOT here (host-process-owned).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LmvMetrics {
    /// Caller sets `sizeof`; core stamps the byte count it actually wrote.
    pub struct_size: u32,
    /// Equals [`lmv_abi_version`].
    pub abi_version: u32,
    /// Frames per second over the rolling window.
    pub fps: f32,
    /// Mean frame time (ms).
    pub frame_ms_avg: f32,
    /// 99th-percentile frame time (ms).
    pub frame_ms_p99: f32,
    /// Frames recorded since creation.
    pub frames_total: u64,
    /// Frames the renderer skipped.
    pub frames_dropped: u64,
    /// Core-tracked GPU resource bytes (approximate; wgpu exposes no device mem).
    pub gpu_bytes: u64,
    /// Draw calls on the last frame.
    pub draw_calls: u32,
    /// Reserved, always 0.
    pub reserved: u32,
}

/// Same headroom as the standalone capture path (~340 ms @ 48 kHz).
const RING_CAPACITY_FRAMES: usize = 16_384;

/// Read the `LMV_DEBUG_OVERLAY` env var once and map a truthy value to the
/// overlay flag (a boundary read at create time).
fn debug_flags_from_env() -> u32 {
    match std::env::var(DEBUG_OVERLAY_ENV) {
        Ok(v) => {
            let v = v.trim().to_ascii_lowercase();
            if matches!(v.as_str(), "1" | "true" | "on" | "yes") {
                LMV_DEBUG_OVERLAY
            } else {
                LMV_DEBUG_OFF
            }
        }
        Err(_) => LMV_DEBUG_OFF,
    }
}

struct RenderState {
    consumer: SampleConsumer,
    analyzer: Analyzer,
    renderer: Option<Renderer>,
    scratch: Vec<f32>,
    /// Presets from `lmv_load_presets` called before a window was attached;
    /// installed on the renderer as soon as `lmv_attach_window` creates it.
    /// Empty once installed (or if load always followed attach).
    pending_presets: Vec<Preset>,
    /// Debug flags (ADR-0008), seeded from the env at create and updatable via
    /// `lmv_set_debug`. Held here so a set-before-attach persists, then applied
    /// to the renderer when `lmv_attach_window` creates it.
    debug_flags: u32,
}

/// Opaque to C. The two `UnsafeCell`s implement the documented two-thread
/// contract without locks: each cell is touched by exactly one thread role.
pub struct LmvHandle {
    channels: u16,
    producer: UnsafeCell<SampleProducer>,
    render: UnsafeCell<RenderState>,
}

/// The ABI version this build implements (see [`LMV_ABI_VERSION`]).
#[unsafe(no_mangle)]
pub extern "C" fn lmv_abi_version() -> u32 {
    LMV_ABI_VERSION
}

/// Create a visualizer for the given stream format. Returns null if the
/// format is rejected (see the bounds in the header) or on internal failure.
#[unsafe(no_mangle)]
pub extern "C" fn lmv_create(sample_rate: u32, channels: u16) -> *mut LmvHandle {
    catch_unwind(|| {
        let format = AudioFormat {
            sample_rate,
            channels,
        };
        let Ok((producer, consumer)) = intake(format, RING_CAPACITY_FRAMES) else {
            return std::ptr::null_mut();
        };
        let Ok(analyzer) = Analyzer::new(format) else {
            return std::ptr::null_mut();
        };
        Box::into_raw(Box::new(LmvHandle {
            channels,
            producer: UnsafeCell::new(producer),
            render: UnsafeCell::new(RenderState {
                consumer,
                analyzer,
                renderer: None,
                scratch: vec![0.0; 32_768],
                pending_presets: Vec::new(),
                debug_flags: debug_flags_from_env(),
            }),
        }))
    })
    .unwrap_or(std::ptr::null_mut())
}

/// Destroy the handle. No other call may race this or use the handle after.
///
/// # Safety
/// `handle` must be a pointer returned by `lmv_create`, not yet freed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lmv_free(handle: *mut LmvHandle) {
    if handle.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| {
        drop(unsafe { Box::from_raw(handle) });
    }));
}

/// Push `sample_count` interleaved f32 samples (must be whole frames).
/// Real-time safe; excess is dropped if the ring is full.
///
/// # Safety
/// `handle` valid per `lmv_create`; `samples` points at `sample_count` f32s.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lmv_push_samples(
    handle: *mut LmvHandle,
    samples: *const f32,
    sample_count: u32,
) -> i32 {
    if handle.is_null() || samples.is_null() {
        return LMV_ERR_INVALID_ARG;
    }
    let handle = unsafe { &*handle };
    if !sample_count.is_multiple_of(handle.channels as u32) {
        return LMV_ERR_INVALID_ARG;
    }
    catch_unwind(AssertUnwindSafe(|| {
        let slice = unsafe { std::slice::from_raw_parts(samples, sample_count as usize) };
        // Safety: the documented contract gives this thread exclusive use of
        // the producer cell.
        let producer = unsafe { &mut *handle.producer.get() };
        producer.push_samples(slice);
        LMV_OK
    }))
    .unwrap_or(LMV_ERR_PANIC)
}

/// Attach the native window to render into (Win32 HWND on Windows — the
/// only supported host platform for now). Call once, from the render thread.
///
/// # Safety
/// `handle` valid per `lmv_create`; `hwnd` a valid window handle outliving
/// this visualizer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lmv_attach_window(
    handle: *mut LmvHandle,
    hwnd: *mut std::ffi::c_void,
    width: u32,
    height: u32,
) -> i32 {
    if handle.is_null() {
        return LMV_ERR_INVALID_ARG;
    }
    #[cfg(not(windows))]
    {
        let _ = (hwnd, width, height);
        LMV_ERR_UNSUPPORTED
    }
    #[cfg(windows)]
    {
        let handle = unsafe { &*handle };
        let Some(hwnd) = std::num::NonZeroIsize::new(hwnd as isize) else {
            return LMV_ERR_INVALID_ARG;
        };
        catch_unwind(AssertUnwindSafe(|| {
            match unsafe { Renderer::new_from_win32_hwnd(hwnd, width, height) } {
                Ok(mut renderer) => {
                    let state = unsafe { &mut *handle.render.get() };
                    // Apply any presets loaded before the window existed.
                    if !state.pending_presets.is_empty() {
                        renderer.set_presets(std::mem::take(&mut state.pending_presets));
                    }
                    // Always collect metrics (so the host can poll its log); the
                    // overlay follows the seeded/updated debug flags (ADR-0008).
                    renderer.enable_diagnostics(true);
                    renderer.set_overlay(state.debug_flags & LMV_DEBUG_OVERLAY != 0);
                    state.renderer = Some(renderer);
                    LMV_OK
                }
                Err(_) => LMV_ERR_RENDER,
            }
        }))
        .unwrap_or(LMV_ERR_PANIC)
    }
}

/// Drain pending samples through analysis and draw one frame into the
/// attached window.
///
/// # Safety
/// `handle` valid per `lmv_create`; render-thread role only.
#[unsafe(no_mangle)]
#[allow(
    clippy::indexing_slicing,
    reason = "n = pop_samples(&mut scratch) <= scratch.len(), so scratch[..n] is in range"
)]
pub unsafe extern "C" fn lmv_render(handle: *mut LmvHandle) -> i32 {
    if handle.is_null() {
        return LMV_ERR_INVALID_ARG;
    }
    let handle = unsafe { &*handle };
    catch_unwind(AssertUnwindSafe(|| {
        let state = unsafe { &mut *handle.render.get() };
        let Some(renderer) = state.renderer.as_mut() else {
            return LMV_ERR_NO_WINDOW;
        };
        loop {
            let n = state.consumer.pop_samples(&mut state.scratch);
            if n == 0 {
                break;
            }
            state.analyzer.push_interleaved(&state.scratch[..n]);
        }
        let frame = state.analyzer.take_frame();
        // Legacy fixed 1/60 s step (Plan 0014 Phase 2). ABI v4 adds
        // `lmv_render_dt` for host-measured real time (Phase 5); this entry then
        // becomes its exact 1/60 wrapper.
        match renderer.render(&frame, 1.0 / 60.0) {
            Ok(()) => LMV_OK,
            Err(_) => LMV_ERR_RENDER,
        }
    }))
    .unwrap_or(LMV_ERR_PANIC)
}

/// Tell the renderer the window was resized.
///
/// # Safety
/// `handle` valid per `lmv_create`; render-thread role only.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lmv_resize(handle: *mut LmvHandle, width: u32, height: u32) -> i32 {
    if handle.is_null() {
        return LMV_ERR_INVALID_ARG;
    }
    let handle = unsafe { &*handle };
    catch_unwind(AssertUnwindSafe(|| {
        let state = unsafe { &mut *handle.render.get() };
        match state.renderer.as_mut() {
            Some(renderer) => {
                renderer.resize(width, height);
                LMV_OK
            }
            None => LMV_ERR_NO_WINDOW,
        }
    }))
    .unwrap_or(LMV_ERR_PANIC)
}

/// Switch to the next scene (same roster as the standalone — parity by
/// construction).
///
/// # Safety
/// `handle` valid per `lmv_create`; render-thread role only.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lmv_cycle_scene(handle: *mut LmvHandle) -> i32 {
    if handle.is_null() {
        return LMV_ERR_INVALID_ARG;
    }
    let handle = unsafe { &*handle };
    catch_unwind(AssertUnwindSafe(|| {
        let state = unsafe { &mut *handle.render.get() };
        match state.renderer.as_mut() {
            Some(renderer) => {
                renderer.cycle_preset();
                LMV_OK
            }
            None => LMV_ERR_NO_WINDOW,
        }
    }))
    .unwrap_or(LMV_ERR_PANIC)
}

/// Seed `path` with the embedded curated presets (writing only files that are
/// absent, never overwriting), then load every valid preset found there and
/// install it as this handle's preset set. Returns the number of presets loaded
/// (`>= 0`), or a negative `LMV_ERR_*` code (null handle/path, invalid UTF-8).
/// A directory with no valid presets keeps the current set (degrade, never
/// crash). The seed step is idempotent — safe to call on every host start. If
/// no window is attached yet, the loaded set is applied when `lmv_attach_window`
/// next creates the renderer.
///
/// # Safety
/// `handle` valid per `lmv_create`; `path_utf8` points at `path_len` bytes of
/// UTF-8 text. Render-thread role only.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lmv_load_presets(
    handle: *mut LmvHandle,
    path_utf8: *const u8,
    path_len: usize,
) -> i32 {
    if handle.is_null() || path_utf8.is_null() {
        return LMV_ERR_INVALID_ARG;
    }
    let handle = unsafe { &*handle };
    catch_unwind(AssertUnwindSafe(|| {
        let bytes = unsafe { std::slice::from_raw_parts(path_utf8, path_len) };
        let Ok(path_str) = std::str::from_utf8(bytes) else {
            return LMV_ERR_INVALID_ARG;
        };
        let path = std::path::Path::new(path_str);
        // Seeding is best-effort: a read-only or otherwise unusable directory
        // still loads whatever is already present rather than failing the call.
        let _ = preset::seed_dir(path);
        let report = preset::load_dir(path);
        let count = report.presets.len() as i32;
        let state = unsafe { &mut *handle.render.get() };
        match state.renderer.as_mut() {
            // set_presets ignores an empty set, so an empty dir keeps the
            // current roster.
            Some(renderer) => renderer.set_presets(report.presets),
            None => state.pending_presets = report.presets,
        }
        count
    }))
    .unwrap_or(LMV_ERR_PANIC)
}

/// Set the debug flag set on the handle (`LMV_DEBUG_*`; higher bits reserved and
/// ignored). Idempotent and cheap — callable from the render-thread role at any
/// time, including before a window is attached (the flags are applied when the
/// renderer is created). Added in ABI v3 (ADR-0008).
///
/// # Safety
/// `handle` valid per `lmv_create`; render-thread role only.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lmv_set_debug(handle: *mut LmvHandle, flags: u32) -> i32 {
    if handle.is_null() {
        return LMV_ERR_INVALID_ARG;
    }
    let handle = unsafe { &*handle };
    catch_unwind(AssertUnwindSafe(|| {
        let state = unsafe { &mut *handle.render.get() };
        state.debug_flags = flags;
        if let Some(renderer) = state.renderer.as_mut() {
            renderer.set_overlay(flags & LMV_DEBUG_OVERLAY != 0);
        }
        LMV_OK
    }))
    .unwrap_or(LMV_ERR_PANIC)
}

/// Fill `out` (caller-allocated) with the current diagnostics snapshot. The
/// caller sets `out->struct_size = sizeof(LmvMetrics)` first; the core writes at
/// most that many bytes and stamps the `struct_size`/`abi_version` it wrote, so
/// an older host reading a newer core still reads its prefix safely. No
/// allocation crosses the boundary. Returns `LMV_OK`, or `LMV_ERR_INVALID_ARG`
/// on a null handle/out. Added in ABI v3 (ADR-0008).
///
/// # Safety
/// `handle` valid per `lmv_create`; `out` points at a caller-allocated
/// `LmvMetrics` whose `struct_size` field is initialized. Render-thread role only.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn lmv_get_metrics(handle: *mut LmvHandle, out: *mut LmvMetrics) -> i32 {
    if handle.is_null() || out.is_null() {
        return LMV_ERR_INVALID_ARG;
    }
    let handle = unsafe { &*handle };
    catch_unwind(AssertUnwindSafe(|| {
        let state = unsafe { &mut *handle.render.get() };
        // Core-only values; zeros (a valid, versioned snapshot) before a window
        // exists, since the metrics live on the renderer.
        let m = state
            .renderer
            .as_ref()
            .map(|r| r.metrics())
            .unwrap_or_default();

        let full = std::mem::size_of::<LmvMetrics>() as u32;
        // Honor the caller's declared buffer size for forward compatibility.
        let caller = unsafe { (*out).struct_size };
        let n = if caller == 0 { full } else { caller.min(full) };

        let local = LmvMetrics {
            struct_size: n,
            abi_version: LMV_ABI_VERSION,
            fps: m.fps,
            frame_ms_avg: m.frame_ms_avg,
            frame_ms_p99: m.frame_ms_p99,
            frames_total: m.frames_total,
            frames_dropped: m.frames_dropped,
            gpu_bytes: m.gpu_bytes,
            draw_calls: m.draw_calls,
            reserved: 0,
        };
        // Write only the first `n` bytes the caller has room for.
        unsafe {
            std::ptr::copy_nonoverlapping(
                std::ptr::from_ref(&local).cast::<u8>(),
                out.cast::<u8>(),
                n as usize,
            );
        }
        LMV_OK
    }))
    .unwrap_or(LMV_ERR_PANIC)
}
