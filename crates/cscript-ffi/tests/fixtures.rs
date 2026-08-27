//! Compiles the `.c` fixtures in `tests/fixtures/` into shared libraries at test time (via the
//! system `cc`, exactly the build step a real user would run themselves), then loads and calls
//! them through `cscript-ffi` -- an end-to-end check of the whole load/start/output/free
//! lifecycle against a real compiled `.so`, not just a self-consistency check against Rust-side
//! mocks.

use std::path::{Path, PathBuf};
use std::process::Command;

use cscript_ffi::CScriptRegistry;

fn compile_fixture(name: &str) -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = manifest_dir
        .join("tests/fixtures")
        .join(format!("{name}.c"));
    let out_dir = std::env::temp_dir().join("cscript-ffi-test-fixtures");
    std::fs::create_dir_all(&out_dir).expect("create fixture output dir");
    let lib_path = out_dir.join(format!("lib{name}.so"));

    let status = Command::new("cc")
        .args(["-shared", "-fPIC", "-O0", "-o"])
        .arg(&lib_path)
        .arg(&source)
        .status()
        .expect("run cc to compile fixture");
    assert!(status.success(), "cc failed to compile fixture {name}");
    lib_path
}

#[test]
fn stateless_fixture_has_no_free_symbol_and_still_works() {
    let lib_path = compile_fixture("stateless_gain");
    let mut registry = CScriptRegistry::new();
    let mut instance = registry
        .instantiate(&lib_path)
        .expect("load stateless fixture");

    let out = instance.call(&[21.0], 0.0, 1);
    assert_eq!(out, vec![42.0]);
    // Called again: since this block truly carries no state, the result must not depend on
    // call history.
    let out2 = instance.call(&[3.0], 0.0, 1);
    assert_eq!(out2, vec![6.0]);
}

#[test]
fn stateful_fixture_accumulates_across_calls() {
    let lib_path = compile_fixture("accumulator");
    let mut registry = CScriptRegistry::new();
    let mut instance = registry
        .instantiate(&lib_path)
        .expect("load accumulator fixture");

    let out1 = instance.call(&[1.0], 0.0, 2);
    assert_eq!(out1, vec![1.0, 1.0]); // sum=1, count=1
    let out2 = instance.call(&[2.0], 0.0, 2);
    assert_eq!(out2, vec![3.0, 2.0]); // sum=1+2=3, count=2
    let out3 = instance.call(&[4.0], 0.0, 2);
    assert_eq!(out3, vec![7.0, 3.0]); // sum=3+4=7, count=3
}

#[test]
fn two_instances_of_the_same_library_keep_independent_state() {
    let lib_path = compile_fixture("accumulator");
    let mut registry = CScriptRegistry::new();
    let mut a = registry.instantiate(&lib_path).expect("load instance a");
    let mut b = registry.instantiate(&lib_path).expect("load instance b");

    a.call(&[10.0], 0.0, 2);
    a.call(&[10.0], 0.0, 2);
    let a_out = a.call(&[10.0], 0.0, 2);
    assert_eq!(a_out, vec![30.0, 3.0]);

    // b has never been called before: its own state must start fresh, not inherit a's.
    let b_out = b.call(&[1.0], 0.0, 2);
    assert_eq!(b_out, vec![1.0, 1.0]);
}

#[test]
fn clone_produces_independent_state() {
    let lib_path = compile_fixture("accumulator");
    let mut registry = CScriptRegistry::new();
    let mut original = registry
        .instantiate(&lib_path)
        .expect("load accumulator fixture");

    original.call(&[5.0], 0.0, 2);
    original.call(&[5.0], 0.0, 2); // original: sum=10, count=2

    assert!(original.supports_clone());
    let mut cloned = original
        .try_clone()
        .expect("cscript_clone must be exported");

    // Diverge each independently -- neither should see the other's subsequent calls.
    let original_out = original.call(&[1.0], 0.0, 2);
    let cloned_out = cloned.call(&[100.0], 0.0, 2);

    assert_eq!(original_out, vec![11.0, 3.0]); // 10+1, count 3
    assert_eq!(cloned_out, vec![110.0, 3.0]); // 10+100, count 3 (cloned at sum=10,count=2)
}

#[test]
fn stateless_fixture_without_cscript_clone_reports_unsupported_not_panic() {
    let lib_path = compile_fixture("stateless_gain");
    let mut registry = CScriptRegistry::new();
    let instance = registry
        .instantiate(&lib_path)
        .expect("load stateless fixture");

    assert!(!instance.supports_clone());
    assert!(instance.try_clone().is_none());
}

#[test]
#[should_panic(expected = "cscript_clone")]
fn clone_trait_panics_when_cscript_clone_is_missing() {
    let lib_path = compile_fixture("stateless_gain");
    let mut registry = CScriptRegistry::new();
    let instance = registry
        .instantiate(&lib_path)
        .expect("load stateless fixture");

    let _ = instance.clone();
}

#[test]
fn rk4_step_xc_matches_the_closed_form_exponential_decay() {
    // dx/dt = -K*x, x0 = 1.0, K = 2.0 (see decay_xc.c): x(t) = exp(-K*t). Integrated in 200
    // steps of dt=1e-3 (t_final=0.2), against a real compiled .so, not a Rust-side mock of the
    // derivative function -- an end-to-end check of the whole xc contract, not just the RK4
    // arithmetic in isolation.
    let lib_path = compile_fixture("decay_xc");
    let mut registry = CScriptRegistry::new();
    let instance = registry
        .instantiate_xc(&lib_path)
        .expect("load decay_xc fixture via the xc contract");

    let k = 2.0_f64;
    let dt = 1e-3;
    let mut xc = vec![1.0_f64];
    let mut t = 0.0_f64;
    for _ in 0..200 {
        xc = instance.rk4_step_xc(&xc, &[0.0], dt);
        t += dt;
    }
    let expected = (-k * t).exp();
    assert!(
        (xc[0] - expected).abs() < 1e-9,
        "xc={}, expected exp(-K*t)={} at t={t}",
        xc[0],
        expected
    );
}

#[test]
fn call_xc_reads_the_given_xc_and_still_mutates_its_own_xd_state() {
    let lib_path = compile_fixture("decay_xc");
    let mut registry = CScriptRegistry::new();
    let mut instance = registry
        .instantiate_xc(&lib_path)
        .expect("load decay_xc fixture via the xc contract");

    let out1 = instance.call_xc(&[0.0], 1e-3, &[0.5], 2);
    assert_eq!(out1, vec![0.5, 1.0]); // out[0] = xc[0] passed in; out[1] = call count (xd)
    let out2 = instance.call_xc(&[0.0], 1e-3, &[0.25], 2);
    assert_eq!(out2, vec![0.25, 2.0]); // xd keeps counting across calls, independent of xc
}

#[test]
fn instantiate_rejects_an_xc_only_library_and_instantiate_xc_rejects_a_plain_one() {
    // stateless_gain only exports cscript_output -- the plain contract -- so instantiate_xc
    // must reject it (missing cscript_derivative), the mirror image of
    // missing_required_symbol_is_a_clear_error_not_a_panic below.
    let plain_lib = compile_fixture("stateless_gain");
    let mut registry = CScriptRegistry::new();
    let err = registry
        .instantiate_xc(&plain_lib)
        .expect_err("a plain cscript_output-only library must not satisfy the xc contract");
    assert!(
        err.to_string().contains("cscript_derivative"),
        "error should name the missing symbol, got: {err}"
    );

    // decay_xc only exports cscript_output_xc/cscript_derivative -- the xc contract -- so
    // plain instantiate must reject it (missing cscript_output).
    let xc_lib = compile_fixture("decay_xc");
    let mut registry2 = CScriptRegistry::new();
    let err2 = registry2
        .instantiate(&xc_lib)
        .expect_err("an xc-only library must not satisfy the plain cscript_output contract");
    assert!(
        err2.to_string().contains("cscript_output"),
        "error should name the missing symbol, got: {err2}"
    );
}

#[test]
fn missing_required_symbol_is_a_clear_error_not_a_panic() {
    // A block exporting only cscript_start, to exercise the MissingSymbol path for a
    // genuinely required symbol (cscript_output) -- generated straight into the temp output
    // dir, not tests/fixtures/, since this source is synthesized by the test itself rather
    // than a real fixture meant to be read/reused.
    let out_dir = std::env::temp_dir().join("cscript-ffi-test-fixtures");
    std::fs::create_dir_all(&out_dir).expect("create fixture output dir");
    let source = out_dir.join("broken_missing_output.c");
    std::fs::write(&source, "void *cscript_start(void) { return (void*)0; }\n")
        .expect("write broken fixture source");
    let lib_path = out_dir.join("libbroken_missing_output.so");
    let status = Command::new("cc")
        .args(["-shared", "-fPIC", "-O0", "-o"])
        .arg(&lib_path)
        .arg(&source)
        .status()
        .expect("run cc to compile broken fixture");
    assert!(status.success());

    let mut registry = CScriptRegistry::new();
    let err = registry
        .instantiate(&lib_path)
        .expect_err("missing cscript_output must be reported, not panic");
    let message = err.to_string();
    assert!(
        message.contains("cscript_output"),
        "error should name the missing symbol, got: {message}"
    );
}
