//! Plan 0003 Phase 4: the pure expression evaluator and TOML preset schema.
//! Values are exact, functions behave, malformed input is rejected without a
//! panic, compiled evaluation allocates nothing, and a sample preset parses
//! with its bindings intact.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use lmv_core::preset::{Preset, SystemKind, Variables, compile};

/// Global allocator that counts allocation calls, so a test can assert that a
/// region performs no heap allocation.
struct Counting;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::SeqCst);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::SeqCst);
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::SeqCst);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// All-zero variables except where overridden per test.
fn vars(bass: f32, mid: f32, treb: f32, onset: f32, beat: f32, bar: f32, time: f32) -> Variables {
    Variables::new(bass, mid, treb, onset, beat, bar, time)
}

#[test]
fn arithmetic_evaluates_exactly() {
    let e = compile("bass * 2 + 0.1").expect("compiles");
    let v = vars(0.25, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    // Same f32 operations as the expression, so the result is bit-exact.
    let expected = 0.25f32 * 2.0 + 0.1f32;
    assert_eq!(e.eval(&v), expected);

    // Precedence and parentheses.
    let e = compile("(bass + mid) * 2").expect("compiles");
    let v = vars(1.0, 0.5, 0.0, 0.0, 0.0, 0.0, 0.0);
    assert_eq!(e.eval(&v), 3.0);
}

#[test]
fn builtin_functions_behave() {
    // sin(pi/2) ~ 1
    let e = compile("sin(time)").expect("compiles");
    let v = vars(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, std::f32::consts::FRAC_PI_2);
    assert!((e.eval(&v) - 1.0).abs() < 1e-6);

    // clamp saturates on both sides (and does not panic if lo>hi never occurs).
    let e = compile("clamp(bass, 0, 1)").expect("compiles");
    assert_eq!(e.eval(&vars(2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)), 1.0);
    assert_eq!(e.eval(&vars(-3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)), 0.0);
    assert_eq!(e.eval(&vars(0.4, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)), 0.4);

    // lerp(mid, treb, bar): 2 + (10-2)*0.5 = 6.
    let e = compile("lerp(mid, treb, bar)").expect("compiles");
    let v = vars(0.0, 2.0, 10.0, 0.0, 0.0, 0.5, 0.0);
    assert_eq!(e.eval(&v), 6.0);

    // min/max/abs/floor
    let zero = Variables::default();
    assert_eq!(compile("min(3, 5)").expect("compiles").eval(&zero), 3.0);
    assert_eq!(compile("max(3, 5)").expect("compiles").eval(&zero), 5.0);
    assert_eq!(compile("abs(0 - 4)").expect("compiles").eval(&zero), 4.0);
    assert_eq!(compile("floor(3.9)").expect("compiles").eval(&zero), 3.0);
}

#[test]
fn beat_coerces_as_a_zero_one_value() {
    let e = compile("1.0 + beat * 0.5").expect("compiles");
    assert_eq!(e.eval(&vars(0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0)), 1.5);
    assert_eq!(e.eval(&vars(0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0)), 1.0);
}

#[test]
fn malformed_expressions_fail_to_compile_without_panicking() {
    for bad in [
        "bass * ",     // trailing operator
        "2 +* 3",      // operator where a value is expected
        "nope(1)",     // unknown function
        "unknownvar",  // unknown variable
        "clamp(1, 2)", // wrong arity
        "sin(1, 2)",   // wrong arity
        "1 @ 2",       // illegal character
        "(1 + 2",      // unbalanced parenthesis
        "1 2",         // trailing tokens
        "",            // empty
    ] {
        assert!(
            compile(bad).is_err(),
            "expression {bad:?} should fail to compile"
        );
    }
}

#[test]
fn compiled_eval_performs_no_heap_allocation() {
    let e = compile("clamp(bass * 2 + sin(time), 0, 1) + lerp(mid, treb, bar)").expect("compiles");
    let v = vars(0.5, 0.2, 0.1, 0.0, 1.0, 0.3, 1.23);

    // Warm up (touch any lazy statics before measuring).
    let _ = e.eval(&v);

    let before = ALLOCS.load(Ordering::SeqCst);
    let mut acc = 0.0f32;
    for _ in 0..10_000 {
        acc += e.eval(&v);
    }
    let after = ALLOCS.load(Ordering::SeqCst);

    assert!(acc.is_finite(), "sanity: evaluation produced a real number");
    assert_eq!(
        before,
        after,
        "compiled eval must not allocate; saw {} allocation(s)",
        after - before
    );
}

#[test]
fn sample_preset_parses_with_bindings_intact() {
    let src = r#"
system = "fragment_field"
name = "Test Field"

[params]
warp = "0.3 + bass * 1.5"
hue  = "time * 0.05 + treb"
kick = "beat"
"#;
    let preset = Preset::from_toml_str(src).expect("valid preset");
    assert_eq!(preset.system, SystemKind::FragmentField);
    assert_eq!(preset.name, "Test Field");
    assert_eq!(preset.params.len(), 3);

    let warp = preset
        .params
        .iter()
        .find(|b| b.name == "warp")
        .expect("warp binding present");
    let v = vars(0.2, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
    assert!((warp.expr.eval(&v) - (0.3 + 0.2 * 1.5)).abs() < 1e-6);

    // Name defaults to the system when omitted.
    let unnamed = Preset::from_toml_str("system = \"swarm\"").expect("valid");
    assert_eq!(unnamed.system, SystemKind::Swarm);
    assert_eq!(unnamed.name, "swarm");
    assert!(unnamed.params.is_empty());
}

#[test]
fn bad_presets_are_rejected() {
    // Unknown system.
    assert!(Preset::from_toml_str("system = \"does_not_exist\"").is_err());
    // A parameter with a malformed expression.
    let bad = "system = \"swarm\"\n[params]\nx = \"bass * \"\n";
    assert!(Preset::from_toml_str(bad).is_err());
    // Not even valid TOML.
    assert!(Preset::from_toml_str("system = ").is_err());
}
