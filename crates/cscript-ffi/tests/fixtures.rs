//! Compiles the `.c` fixtures in `tests/fixtures/` into shared libraries at test time (via the
//! system `cc`, exactly the build step a real user would run themselves), then loads and calls
//! them through `cscript-ffi` -- an end-to-end check of the whole load/start/output/free
//! lifecycle against a real compiled `.so`, not just a self-consistency check against Rust-side
//! mocks.
//!
//! Concurrency contract of the compile helpers (GOTCHA-001): the test harness runs every
//! `#[test]` here on its own thread, and several of them ask for the same fixture. Each fixture
//! is therefore compiled at most once per test process (a process-wide registry behind a
//! `Mutex`), into a directory whose name is a hash of the fixture's source and compile flags,
//! and the `.so` is written under a temporary name and `rename`d into place -- so a thread (or
//! another process) that already `dlopen`ed the library never observes a truncated or
//! half-written file. Never `cc -o <final path>` directly: that truncates the file in place.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use cscript_ffi::CScriptRegistry;

const CC_FLAGS: [&str; 4] = ["-shared", "-fPIC", "-O0", "-o"];

/// Fixture name -> compiled library path, for every fixture this process has already built.
fn compiled() -> &'static Mutex<HashMap<String, PathBuf>> {
    static COMPILED: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    COMPILED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Compile `source` (C code) into `lib<name>.so` exactly once per process and return its path.
///
/// The output directory is keyed by a hash of the source text and the compiler flags, so two
/// different sources under the same `name` (or a fixture edited between runs) never share a
/// path, and a library left behind by an earlier process is only ever reused when it was built
/// from byte-identical input.
fn compile_source(name: &str, source: &str) -> PathBuf {
    let mut registry = compiled().lock().expect("fixture registry poisoned");
    if let Some(path) = registry.get(name) {
        return path.clone();
    }

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    CC_FLAGS.hash(&mut hasher);
    source.hash(&mut hasher);
    let out_dir = std::env::temp_dir()
        .join("cscript-ffi-test-fixtures")
        .join(format!("{:016x}", hasher.finish()));
    std::fs::create_dir_all(&out_dir).expect("create fixture output dir");
    let lib_path = out_dir.join(format!("lib{name}.so"));

    if !lib_path.exists() {
        // Unique per process so two concurrent `cargo test` invocations (e.g. two checkouts
        // sharing one TMPDIR) never write the same temporary file either.
        let source_path = out_dir.join(format!("{name}.{}.c", std::process::id()));
        let tmp_path = out_dir.join(format!("lib{name}.so.{}.tmp", std::process::id()));
        std::fs::write(&source_path, source).expect("write fixture source");
        let status = Command::new("cc")
            .args(CC_FLAGS)
            .arg(&tmp_path)
            .arg(&source_path)
            .status()
            .expect("run cc to compile fixture");
        assert!(status.success(), "cc failed to compile fixture {name}");
        // Atomic on POSIX: a reader either sees the old complete inode or the new one.
        std::fs::rename(&tmp_path, &lib_path).expect("move compiled fixture into place");
        let _ = std::fs::remove_file(&source_path);
    }

    registry.insert(name.to_owned(), lib_path.clone());
    lib_path
}

/// Compile `tests/fixtures/<name>.c` (once per process, see [`compile_source`]).
fn compile_fixture(name: &str) -> PathBuf {
    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.c"));
    let source = std::fs::read_to_string(&source_path)
        .unwrap_or_else(|e| panic!("read fixture source {}: {e}", source_path.display()));
    compile_source(name, &source)
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
    // genuinely required symbol (cscript_output) -- synthesized by the test itself rather
    // than kept in tests/fixtures/, since it is not a real fixture meant to be read/reused.
    let lib_path = compile_source(
        "broken_missing_output",
        "void *cscript_start(void) { return (void*)0; }\n",
    );

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

#[test]
fn output_is_read_only_and_update_is_the_sole_place_state_advances() {
    let lib_path = compile_fixture("update_counter");
    let mut registry = CScriptRegistry::new();
    let mut instance = registry
        .instantiate(&lib_path)
        .expect("load update_counter fixture");

    // Calling output() any number of times with no update() must never change the result --
    // output() itself never mutates state in this fixture.
    let out1 = instance.call(&[5.0], 0.0, 1);
    let out2 = instance.call(&[5.0], 0.0, 1);
    let out3 = instance.call(&[5.0], 0.0, 1);
    assert_eq!(out1, vec![0.0]);
    assert_eq!(out2, vec![0.0]);
    assert_eq!(out3, vec![0.0]);

    // update() is the only place the accumulator actually advances.
    instance.update(&[5.0], 0.0, &[]);
    let out4 = instance.call(&[5.0], 0.0, 1);
    assert_eq!(out4, vec![5.0]);

    instance.update(&[3.0], 0.0, &[]);
    let out5 = instance.call(&[5.0], 0.0, 1);
    assert_eq!(out5, vec![8.0]);
}

#[test]
fn update_is_a_silent_no_op_when_the_library_does_not_export_it() {
    // Every existing fixture predates cscript_update -- confirms the new symbol resolution is
    // genuinely optional and doesn't break a library that never heard of it.
    let lib_path = compile_fixture("accumulator");
    let mut registry = CScriptRegistry::new();
    let mut instance = registry
        .instantiate(&lib_path)
        .expect("load accumulator fixture");
    instance.update(&[999.0], 0.0, &[]); // must not panic
    let out = instance.call(&[1.0], 0.0, 2);
    assert_eq!(out, vec![1.0, 1.0]); // unaffected by the no-op update() call above
}

#[test]
fn next_sample_hit_reports_the_library_own_requested_interval() {
    let lib_path = compile_fixture("variable_sample_time");
    let mut registry = CScriptRegistry::new();
    let instance = registry
        .instantiate(&lib_path)
        .expect("load variable_sample_time fixture");

    // Doubles every call: 1, 2, 4, 8 -- exactly what the fixture's own state machine defines,
    // confirming cscript-ffi passes the return value through unmodified.
    assert_eq!(instance.next_sample_hit(&[], &[]), 1.0);
    assert_eq!(instance.next_sample_hit(&[], &[]), 2.0);
    assert_eq!(instance.next_sample_hit(&[], &[]), 4.0);
    assert_eq!(instance.next_sample_hit(&[], &[]), 8.0);
}

#[test]
fn supports_next_sample_hit_is_false_for_every_pre_existing_fixture() {
    // Every fixture predating this feature must correctly report "no" -- confirms the new
    // symbol resolution doesn't accidentally match on something else.
    let lib_path = compile_fixture("accumulator");
    let mut registry = CScriptRegistry::new();
    let instance = registry.instantiate(&lib_path).expect("load fixture");
    assert!(!instance.supports_next_sample_hit());
}

#[test]
fn supports_next_sample_hit_is_true_for_the_variable_sample_time_fixture() {
    let lib_path = compile_fixture("variable_sample_time");
    let mut registry = CScriptRegistry::new();
    let instance = registry.instantiate(&lib_path).expect("load fixture");
    assert!(instance.supports_next_sample_hit());
}

/// The checkpoint state contract, end to end against a real compiled `.so`: the bytes written
/// after a few calls rebuild an instance that continues *exactly* where the original was, and
/// the original is untouched by the read (an independent object, not an alias).
#[test]
fn state_bytes_round_trip_rebuilds_an_identical_independent_state() {
    let lib_path = compile_fixture("accumulator_checkpoint");
    let mut registry = CScriptRegistry::new();
    let mut a = registry.instantiate(&lib_path).expect("load fixture");
    assert!(a.supports_state_io());

    a.call(&[0.1], 0.0, 2);
    a.call(&[0.2], 0.0, 2);
    let bytes = a.state_bytes().expect("state_bytes");
    assert_eq!(bytes.len(), 16, "sizeof(AccState) = double + long");

    let mut b = registry
        .instantiate(&lib_path)
        .expect("load second instance");
    b.restore_state(&bytes).expect("restore_state");
    let from_a = a.call(&[0.4], 0.0, 2);
    let from_b = b.call(&[0.4], 0.0, 2);
    assert_eq!(from_a, from_b);
    assert!(
        from_a[0].to_bits() == (0.1_f64 + 0.2 + 0.4).to_bits(),
        "the sum carries its exact bits, got {:e}",
        from_a[0]
    );
    assert_eq!(from_a[1], 3.0);
    // Independent: stepping b again leaves a where it was.
    b.call(&[100.0], 0.0, 2);
    assert_eq!(a.call(&[0.0], 0.0, 2)[1], 4.0);
}

/// Exporting only some of the three symbols is a load-time error naming the missing one --
/// not a silent "this library does not checkpoint".
#[test]
fn a_partial_state_contract_is_rejected_at_load_naming_the_missing_symbol() {
    let lib_path = compile_fixture("accumulator_partial_state");
    let mut registry = CScriptRegistry::new();
    let err = registry
        .instantiate(&lib_path)
        .expect_err("a partial state contract must not load");
    assert!(
        matches!(
            &err,
            cscript_ffi::CScriptError::MissingSymbol { symbol, .. }
                if *symbol == "cscript_state_read"
        ),
        "expected MissingSymbol naming cscript_state_read, got {err:?}"
    );
    assert!(err.to_string().contains("all-or-nothing"));
}

/// A library exporting none of the three still loads and runs; the state calls report the
/// absence as an ordinary error rather than a panic.
#[test]
fn no_state_contract_reports_unsupported_not_panic() {
    let lib_path = compile_fixture("accumulator");
    let mut registry = CScriptRegistry::new();
    let mut instance = registry.instantiate(&lib_path).expect("load fixture");
    assert!(!instance.supports_state_io());
    assert!(matches!(
        instance.state_bytes(),
        Err(cscript_ffi::CScriptError::StateContractNotExported { .. })
    ));
    assert!(matches!(
        instance.restore_state(&[]),
        Err(cscript_ffi::CScriptError::StateContractNotExported { .. })
    ));
    assert_eq!(instance.call(&[1.0], 0.0, 2), vec![1.0, 1.0]);
}
