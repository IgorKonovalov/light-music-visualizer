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

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define LMV_ABI_VERSION 1u

/* Result codes (0 success, negative failure). */
#define LMV_OK 0
#define LMV_ERR_INVALID_ARG (-1)
#define LMV_ERR_FORMAT (-2)
#define LMV_ERR_RENDER (-3)
#define LMV_ERR_NO_WINDOW (-4)
#define LMV_ERR_PANIC (-5)
#define LMV_ERR_UNSUPPORTED (-6)

/* Opaque visualizer instance. */
typedef struct LmvHandle LmvHandle;

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

/* Analyze pending audio and draw one frame. Call at display cadence. */
int32_t lmv_render(LmvHandle *handle);

/* Notify of a window client-size change (physical pixels). */
int32_t lmv_resize(LmvHandle *handle, uint32_t width, uint32_t height);

/* Advance to the next built-in scene (same roster as the standalone). */
int32_t lmv_cycle_scene(LmvHandle *handle);

#ifdef __cplusplus
} /* extern "C" */
#endif

#endif /* LMV_CORE_H */
