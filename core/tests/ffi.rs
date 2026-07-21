//! First automated coverage of the C ABI (the long-standing zero-CI-coverage
//! gap noted in the Plan 0001/0002 reviews). Drives lmv_create ->
//! lmv_load_presets -> lmv_free across the FFI boundary against a temp dir,
//! confirms the v2 version handshake, and exercises the null-path error path
//! (no UB, documented negative code). No window is attached, so this runs
//! headless: lmv_load_presets stashes the loaded set as pending until a
//! renderer exists, and still reports the loaded count.

use std::path::Path;

use lmv_core::ffi::{
    LMV_ABI_VERSION, LMV_ERR_INVALID_ARG, lmv_abi_version, lmv_create, lmv_free, lmv_load_presets,
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

#[test]
fn abi_version_is_two() {
    assert_eq!(lmv_abi_version(), 2, "runtime ABI version is v2");
    assert_eq!(LMV_ABI_VERSION, 2, "compile-time ABI version is v2");
}
