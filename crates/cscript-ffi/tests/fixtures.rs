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
fn missing_required_symbol_is_a_clear_error_not_a_panic() {
    // Reuse the accumulator source but only export cscript_start, to exercise the
    // MissingSymbol path for a genuinely required symbol (cscript_output).
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = manifest_dir.join("tests/fixtures/broken_missing_output.c");
    std::fs::write(&source, "void *cscript_start(void) { return (void*)0; }\n")
        .expect("write broken fixture source");
    let out_dir = std::env::temp_dir().join("cscript-ffi-test-fixtures");
    std::fs::create_dir_all(&out_dir).expect("create fixture output dir");
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
