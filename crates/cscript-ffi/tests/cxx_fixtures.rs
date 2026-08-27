//! Compiles the `.cpp` fixtures in `tests/fixtures/` (via `cscript.hpp`, this crate's own
//! optional C++ convenience header) into shared libraries at test time with the system
//! `c++`/`g++`, then loads and calls them through `cscript-ffi` exactly like `fixtures.rs`'s own
//! plain-C tests -- confirming a C++-authored `.so` (class-based state, RAII, a
//! compiler-generated copy constructor for `cscript_clone`) is a fully interchangeable
//! `kind=cscript` library, needing zero changes on this crate's own Rust side: `dlopen` and a
//! C-linkage symbol lookup never cared what language produced the `.so` in the first place.

use std::path::{Path, PathBuf};
use std::process::Command;

use cscript_ffi::CScriptRegistry;

fn compile_fixture(name: &str) -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source = manifest_dir
        .join("tests/fixtures")
        .join(format!("{name}.cpp"));
    let out_dir = std::env::temp_dir().join("cscript-ffi-cxx-test-fixtures");
    std::fs::create_dir_all(&out_dir).expect("create fixture output dir");
    let lib_path = out_dir.join(format!("lib{name}.so"));

    let status = Command::new("c++")
        .args(["-std=c++17", "-shared", "-fPIC", "-O0", "-o"])
        .arg(&lib_path)
        .arg(&source)
        .status()
        .expect("run c++ to compile fixture");
    assert!(status.success(), "c++ failed to compile fixture {name}");
    lib_path
}

#[test]
fn cxx_accumulator_matches_the_plain_c_fixtures_own_behavior() {
    // Same expected sequence fixtures.rs's own stateful_fixture_accumulates_across_calls test
    // checks against the C accumulator -- a C++ class with real member state must behave
    // identically, not just "also work."
    let lib_path = compile_fixture("cxx_accumulator");
    let mut registry = CScriptRegistry::new();
    let mut instance = registry
        .instantiate(&lib_path)
        .expect("load cxx_accumulator fixture");

    let out1 = instance.call(&[1.0], 0.0, 2);
    assert_eq!(out1, vec![1.0, 1.0]); // sum=1, count=1
    let out2 = instance.call(&[2.0], 0.0, 2);
    assert_eq!(out2, vec![3.0, 2.0]); // sum=1+2=3, count=2
    let out3 = instance.call(&[4.0], 0.0, 2);
    assert_eq!(out3, vec![7.0, 3.0]); // sum=3+4=7, count=3
}

#[test]
fn two_cxx_instances_of_the_same_library_keep_independent_state() {
    let lib_path = compile_fixture("cxx_accumulator");
    let mut registry = CScriptRegistry::new();
    let mut a = registry.instantiate(&lib_path).expect("instance a");
    let mut b = registry.instantiate(&lib_path).expect("instance b");

    a.call(&[10.0], 0.0, 2);
    a.call(&[10.0], 0.0, 2);
    let out_b = b.call(&[1.0], 0.0, 2);
    // b's own first call must see sum=1, count=1 -- unaffected by a's own two prior calls.
    assert_eq!(out_b, vec![1.0, 1.0]);
}

#[test]
fn cxx_clone_produces_independent_state_via_the_compiler_generated_copy_constructor() {
    // CSCRIPT_EXPORT's cscript_clone is `new ClassName(*state)` -- Accumulator declares no
    // copy constructor of its own, so this is the compiler-generated member-wise copy,
    // confirming that path (not a hand-written one, unlike every C fixture's own cscript_clone)
    // produces a real, independent copy.
    let lib_path = compile_fixture("cxx_accumulator");
    let mut registry = CScriptRegistry::new();
    let mut original = registry.instantiate(&lib_path).expect("load fixture");
    original.call(&[5.0], 0.0, 2); // sum=5, count=1

    let mut cloned = original.clone();
    let out_original = original.call(&[1.0], 0.0, 2); // sum=6, count=2
    let out_cloned = cloned.call(&[100.0], 0.0, 2); // sum=105, count=2

    assert_eq!(out_original, vec![6.0, 2.0]);
    assert_eq!(out_cloned, vec![105.0, 2.0]);
}

/// Not a real test on its own — invoked as a re-exec'd child process by
/// `an_exception_in_output_aborts_the_process_instead_of_unwinding_into_rust` below, gated
/// behind an env var so a normal `cargo test` run never calls into the throwing fixture inline
/// (which would abort the whole test binary, not just this one case). See that test's own doc
/// comment for why a subprocess is needed at all here.
#[test]
fn cxx_throws_helper() {
    if std::env::var("CSCRIPT_CXX_ABORT_HELPER").is_err() {
        return;
    }
    let lib_path = compile_fixture("cxx_throws");
    let mut registry = CScriptRegistry::new();
    let mut instance = registry
        .instantiate(&lib_path)
        .expect("load cxx_throws fixture");
    let _ = instance.call(&[], 0.0, 0); // must abort before returning
    eprintln!("cxx_throws_helper: call() returned instead of aborting -- test bug");
    std::process::exit(2);
}

#[test]
fn an_exception_in_output_aborts_the_process_instead_of_unwinding_into_rust() {
    // Calling into cxx_throws's output() directly here would abort *this* test binary (SIGABRT
    // kills the whole process, not just one #[test]) -- so the actual throwing call happens in
    // a re-exec'd child process (cxx_throws_helper, gated behind CSCRIPT_CXX_ABORT_HELPER), and
    // this test only observes that child's exit status: killed by SIGABRT (signal 6), not a
    // clean exit and not some other crash (segfault, etc.) that would indicate the exception
    // instead escaped into undefined behavior.
    let exe = std::env::current_exe().expect("current test binary path");
    let output = Command::new(exe)
        .args(["--exact", "cxx_throws_helper", "--nocapture"])
        .env("CSCRIPT_CXX_ABORT_HELPER", "1")
        .output()
        .expect("re-exec self to run cxx_throws_helper");

    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        assert_eq!(
            output.status.signal(),
            Some(6), // SIGABRT
            "expected the child to be killed by SIGABRT (std::abort()), got status={:?}, stderr={}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cscript_output"),
        "expected cscript.hpp's own diagnostic naming cscript_output, got: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn cxx_decay_xc_matches_the_closed_form_exponential_decay() {
    // Same closed-form check fixtures.rs's own rk4_step_xc_matches_the_closed_form_exponential_decay
    // test runs against the C decay_xc fixture: dx/dt = -k*x, k=2.0, x(t) = x0*exp(-k*t).
    let lib_path = compile_fixture("cxx_decay_xc");
    let mut registry = CScriptRegistry::new();
    let instance = registry
        .instantiate_xc(&lib_path)
        .expect("load cxx_decay_xc fixture");

    let k = 2.0_f64;
    let dt = 0.01;
    let mut xc = vec![1.0_f64]; // x0 = 1
    let mut t = 0.0_f64;
    for _ in 0..100 {
        xc = instance.rk4_step_xc(&xc, &[0.0], dt);
        t += dt;
    }
    let expected = (-k * t).exp();
    assert!(
        (xc[0] - expected).abs() < 1e-6,
        "xc[0]={}, expected={expected} (t={t})",
        xc[0]
    );
}
