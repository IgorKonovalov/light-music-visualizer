//! First automated coverage of the C ABI (the long-standing zero-CI-coverage
//! gap noted in the Plan 0001/0002 reviews). Drives lmv_create ->
//! lmv_load_presets -> lmv_free across the FFI boundary against a temp dir,
//! confirms the v2 version handshake, and exercises the null-path error path
//! (no UB, documented negative code). No window is attached, so this runs
//! headless: lmv_load_presets stashes the loaded set as pending until a
//! renderer exists, and still reports the loaded count.

use std::path::Path;

use lmv_core::ffi::{
    LMV_ABI_VERSION, LMV_DEBUG_OVERLAY, LMV_ERR_INVALID_ARG, LMV_OK, LmvMetrics, lmv_abi_version,
    lmv_create, lmv_free, lmv_get_metrics, lmv_load_presets, lmv_render, lmv_set_debug,
};

/// Count the `.toml` files in `dir` (0 if it can't be read).
fn toml_count(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "toml"))
                .count()
        })
        .unwrap_or(0)
}

#[test]
fn load_presets_seeds_and_installs_over_the_abi() {
    let dir = std::env::temp_dir().join("lmv_ffi_load_presets_test");
    let _ = std::fs::remove_dir_all(&dir);

    let handle = lmv_create(48_000, 2);
    assert!(
        !handle.is_null(),
        "lmv_create returns a handle for a valid format"
    );

    // Loading against a fresh dir seeds the curated set and installs every
    // valid preset; the return is that count.
    let path = dir.to_str().expect("temp path is valid UTF-8");
    let bytes = path.as_bytes();
    let installed = unsafe { lmv_load_presets(handle, bytes.as_ptr(), bytes.len()) };

    let expected = lmv_core::preset::default_presets().len() as i32;
    assert!(installed > 0, "at least one curated preset installs");
    assert_eq!(
        installed, expected,
        "every embedded curated preset loads over the ABI"
    );
    assert_eq!(
        toml_count(&dir) as i32,
        expected,
        "the temp dir was seeded with the curated files"
    );

    // A null path is rejected with the documented error and no UB.
    let err = unsafe { lmv_load_presets(handle, std::ptr::null(), 0) };
    assert_eq!(err, LMV_ERR_INVALID_ARG, "null path -> invalid arg");

    unsafe { lmv_free(handle) };
    let _ = std::fs::remove_dir_all(&dir);
}

/// Lockstep guard: the Rust `LmvMetrics` must be exactly the 56 bytes the C
/// header's `static_assert(sizeof(LmvMetrics) == 56)` expects. If this breaks,
/// the header's assert would too — fix both together (no cbindgen, ADR-0003).
#[test]
fn lmv_metrics_is_56_bytes() {
    assert_eq!(std::mem::size_of::<LmvMetrics>(), 56);
    assert_eq!(std::mem::align_of::<LmvMetrics>(), 8);
}

#[test]
fn abi_version_is_three() {
    assert_eq!(lmv_abi_version(), 3, "runtime ABI version is v3");
    assert_eq!(LMV_ABI_VERSION, 3, "compile-time ABI version is v3");
}

/// v3 diagnostics ABI (ADR-0008): set the overlay flag and pull a metrics
/// snapshot into a caller-allocated struct, asserting the version + size stamps
/// that guard against a silent Rust/C layout mismatch. Null-arg paths return the
/// documented error with no UB.
///
/// Note: this runs headless (no window), so `lmv_render` returns
/// `LMV_ERR_NO_WINDOW` and the timing fields stay zero — populating them is a
/// windowed runtime check (an attached surface), like the plugin's on-device
/// done-whens. The struct contract this test guards is the silent-memory-bug
/// risk ADR-0008 actually calls out.
#[test]
fn set_debug_and_get_metrics_over_the_abi() {
    let handle = lmv_create(48_000, 2);
    assert!(!handle.is_null(), "lmv_create returns a handle");

    // Toggling the overlay flag is accepted (idempotent, cheap, pre-window).
    let rc = unsafe { lmv_set_debug(handle, LMV_DEBUG_OVERLAY) };
    assert_eq!(rc, LMV_OK, "lmv_set_debug(OVERLAY) -> OK");

    // Pull the snapshot into a caller-allocated, size-declared struct.
    let mut out: LmvMetrics = unsafe { std::mem::zeroed() };
    out.struct_size = std::mem::size_of::<LmvMetrics>() as u32;
    let rc = unsafe { lmv_get_metrics(handle, &mut out) };
    assert_eq!(rc, LMV_OK, "lmv_get_metrics -> OK");
    assert_eq!(out.abi_version, 3, "core stamps the v3 abi_version");
    assert_eq!(
        out.struct_size,
        std::mem::size_of::<LmvMetrics>() as u32,
        "core stamps the bytes it wrote (full struct here)"
    );

    // Headless renders are no-ops (no window); they must not UB or panic.
    for _ in 0..3 {
        let _ = unsafe { lmv_render(handle) };
    }

    // Null-arg error paths: documented negative code, no UB.
    assert_eq!(
        unsafe { lmv_set_debug(std::ptr::null_mut(), LMV_DEBUG_OVERLAY) },
        LMV_ERR_INVALID_ARG,
        "null handle -> invalid arg"
    );
    assert_eq!(
        unsafe { lmv_get_metrics(handle, std::ptr::null_mut()) },
        LMV_ERR_INVALID_ARG,
        "null out -> invalid arg"
    );

    unsafe { lmv_free(handle) };
}
