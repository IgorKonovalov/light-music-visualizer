/*
 * lmv_core.h — C ABI of the light-music-visualizer core (hand-written,
 * kept in lockstep with core/src/ffi.rs).
 *
 * THIS IS A CONTRACT. The C++ host compiles against this header separately
 * from the Rust crate; changing the shape of this surface is an ADR-worthy
 * event. Bump LMV_ABI_VERSION with any such change and check it at runtime
 * via lmv_abi_version().
 *
 * Threading contract:
 *  - lmv_push_samples: at most one calling thread at a time (the host's
 *    audio / visualisation-stream thread). Real-time safe: lock-free, no
 *    allocation, never blocks; excess samples are dropped when the internal
 *    ring is full.
 *  - All other functions: at most one calling thread at a time (the host's
 *    UI/render thread). lmv_create/lmv_free must not race any other call on
 *    the same handle.
 *  - The audio role and the render role may run concurrently.
 */

#ifndef LMV_CORE_H
#define LMV_CORE_H

#include <stddef.h> /* size_t */
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define LMV_ABI_VERSION 4u

/* Result codes (0 success, negative failure). */
#define LMV_OK 0
#define LMV_ERR_INVALID_ARG (-1)
#define LMV_ERR_FORMAT (-2)
#define LMV_ERR_RENDER (-3)
#define LMV_ERR_NO_WINDOW (-4)
#define LMV_ERR_PANIC (-5)
#define LMV_ERR_UNSUPPORTED (-6)

/* Debug flags for lmv_set_debug (ADR-0008). Higher bits reserved, ignored. */
#define LMV_DEBUG_OFF 0u
#define LMV_DEBUG_OVERLAY (1u << 0) /* draw the on-screen diagnostics overlay */

/* Opaque visualizer instance. */
typedef struct LmvHandle LmvHandle;

/*
 * Diagnostics snapshot filled by lmv_get_metrics (ADR-0008). Plain data,
 * caller-allocated - no allocation crosses the ABI. Leads with struct_size +
 * abi_version so later fields append without a version bump: the caller sets
 * struct_size = sizeof(LmvMetrics), the core writes at most that many bytes and
 * stamps what it wrote. Process RSS is deliberately NOT here (host-process
 * owned; each shell reads its own). Layout mirrors the Rust #[repr(C)] struct in
 * core/src/ffi.rs - keep the two in lockstep. Added in ABI v3.
 */
typedef struct LmvMetrics {
    uint32_t struct_size;   /* caller sets sizeof; core stamps what it wrote */
    uint32_t abi_version;   /* == lmv_abi_version() */
    float fps;
    float frame_ms_avg;
    float frame_ms_p99;
    uint64_t frames_total;
    uint64_t frames_dropped;
    uint64_t gpu_bytes;     /* core-tracked GPU bytes (approx; no device mem) */
    uint32_t draw_calls;    /* last frame */
    uint32_t reserved;      /* always 0 */
} LmvMetrics;

#ifdef __cplusplus
/* A layout mismatch with the Rust struct is a silent memory bug, not a compile
 * error (no cbindgen, per ADR-0003); guard it where the C++ shim compiles. */
static_assert(sizeof(LmvMetrics) == 56, "LmvMetrics layout must match core/src/ffi.rs");
#endif

/* Runtime ABI version of the linked core; compare with LMV_ABI_VERSION. */
uint32_t lmv_abi_version(void);

/*
 * Create a visualizer for one PCM stream. Accepted bounds: sample_rate in
 * [8000, 384000], channels in [1, 8]. Returns NULL on rejection or failure.
 */
LmvHandle *lmv_create(uint32_t sample_rate, uint16_t channels);

/* Destroy. The handle must not be used afterwards. NULL is a no-op. */
void lmv_free(LmvHandle *handle);

/*
 * Push interleaved 32-bit float samples. sample_count is the number of
 * floats and must be a whole number of frames (multiple of channels).
 */
int32_t lmv_push_samples(LmvHandle *handle, const float *samples,
                         uint32_t sample_count);

/*
 * Attach the native window to render into, with its current client size in
 * physical pixels. On Windows pass the HWND. The window must outlive the
 * handle (or be detached by freeing the handle first).
 */
int32_t lmv_attach_window(LmvHandle *handle, void *hwnd, uint32_t width,
                          uint32_t height);

/* Analyze pending audio and draw one frame. Call at display cadence. Exactly
 * equivalent to lmv_render_dt(handle, 1.0f / 60.0f) - the fixed-step wrapper for
 * a host that has no real elapsed time to supply. */
int32_t lmv_render(LmvHandle *handle);

/*
 * Analyze pending audio and draw one frame, advancing the simulation by
 * dt_seconds of real time. Call at display cadence with the measured elapsed
 * time since the previous frame, so a feedback simulation runs at the same
 * wall-clock rate on any refresh; core never reads a clock. lmv_render is the
 * 1/60 s wrapper over this. Added in ABI v4.
 */
int32_t lmv_render_dt(LmvHandle *handle, float dt_seconds);

/* Notify of a window client-size change (physical pixels). */
int32_t lmv_resize(LmvHandle *handle, uint32_t width, uint32_t height);

/* Advance to the next built-in scene (same roster as the standalone). */
int32_t lmv_cycle_scene(LmvHandle *handle);

/*
 * Seed `path_utf8` (a directory, `path_len` bytes of UTF-8, not
 * NUL-terminated) with the embedded curated presets, writing only files that
 * are absent (never overwriting user edits), then load every valid preset
 * found there and install it as this handle's preset set. lmv_cycle_scene then
 * cycles the loaded set. Returns the number of presets loaded (>= 0), or a
 * negative LMV_ERR_* (invalid arg on a null handle/path or non-UTF-8 path). A
 * directory with no valid presets keeps the current set. The seed step is
 * idempotent - safe to call once on every host start. Added in ABI v2.
 */
int32_t lmv_load_presets(LmvHandle *handle, const uint8_t *path_utf8,
                         size_t path_len);

/*
 * Set the debug flag set on the handle (LMV_DEBUG_*). Idempotent and cheap;
 * callable at any time from the render-thread role, including before a window is
 * attached (the flags apply when the renderer is created). LMV_DEBUG_OVERLAY at
 * create time can also be seeded from the LMV_DEBUG_OVERLAY environment
 * variable. Added in ABI v3.
 */
int32_t lmv_set_debug(LmvHandle *handle, uint32_t flags);

/*
 * Fill *out (caller-allocated) with the current diagnostics snapshot. Set
 * out->struct_size = sizeof(LmvMetrics) before calling. Returns LMV_OK, or
 * LMV_ERR_INVALID_ARG on a null handle/out. No allocation crosses the ABI; safe
 * to poll every frame or once a second. Added in ABI v3.
 */
int32_t lmv_get_metrics(LmvHandle *handle, LmvMetrics *out);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* LMV_CORE_H */
