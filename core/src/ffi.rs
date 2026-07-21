//! The versioned C ABI — the single FFI seam of the project (ADR-0001).
//!
//! **This surface is a contract.** The C++ foobar2000 shim compiles against
//! `core/include/lmv_core.h` separately from this crate, so any change to the
//! shape of these functions is an ADR-worthy event, not a casual edit. Keep
//! it minimal: create/free, push samples, attach window, render, resize,
//! cycle scene, version query.
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

use std::cell::UnsafeCell;
use std::panic::{AssertUnwindSafe, catch_unwind};

use crate::audio::{AudioFormat, SampleConsumer, SampleProducer, intake};
use crate::dsp::Analyzer;
use crate::render::Renderer;

/// Bump on any ABI shape change (with the accompanying ADR).
pub const LMV_ABI_VERSION: u32 = 1;

pub const LMV_OK: i32 = 0;
pub const LMV_ERR_INVALID_ARG: i32 = -1;
pub const LMV_ERR_FORMAT: i32 = -2;
pub const LMV_ERR_RENDER: i32 = -3;
pub const LMV_ERR_NO_WINDOW: i32 = -4;
pub const LMV_ERR_PANIC: i32 = -5;
pub const LMV_ERR_UNSUPPORTED: i32 = -6;

/// Same headroom as the standalone capture path (~340 ms @ 48 kHz).
const RING_CAPACITY_FRAMES: usize = 16_384;

struct RenderState {
    consumer: SampleConsumer,
    analyzer: Analyzer,
    renderer: Option<Renderer>,
    scratch: Vec<f32>,
}

/// Opaque to C. The two `UnsafeCell`s implement the documented two-thread
/// contract without locks: each cell is touched by exactly one thread role.
pub struct LmvHandle {
    channels: u16,
    producer: UnsafeCell<SampleProducer>,
    render: UnsafeCell<RenderState>,
}

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
                Ok(renderer) => {
                    let state = unsafe { &mut *handle.render.get() };
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
        match renderer.render(&frame) {
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
                renderer.cycle_scene();
                LMV_OK
            }
            None => LMV_ERR_NO_WINDOW,
        }
    }))
    .unwrap_or(LMV_ERR_PANIC)
}
