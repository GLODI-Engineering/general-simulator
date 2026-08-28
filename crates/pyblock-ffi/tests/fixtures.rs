//! Loads and calls the `.py` fixtures in `tests/fixtures/` through the real `pyblock-ffi`
//! API -- the direct Python-side analog of `cscript-ffi`'s own `tests/fixtures.rs`, exercising
//! the same load/start/output/clone lifecycle plus the `xc` contract, vector inputs, and
//! Python-exception propagation.

use std::path::{Path, PathBuf};

use pyblock_ffi::{PyBlockError, PyBlockRegistry, PyInput};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn stateless_fixture_has_no_persistent_state_and_still_works() {
    let mut registry = PyBlockRegistry::new();
    let mut instance = registry
        .instantiate(&fixture("gain.py"))
        .expect("load gain fixture");

    let out = instance
        .call(0.0, 1e-5, &[PyInput::Scalar(21.0)], 1)
        .unwrap();
    assert_eq!(out, vec![42.0]);
    let out2 = instance
        .call(0.0, 1e-5, &[PyInput::Scalar(3.0)], 1)
        .unwrap();
    assert_eq!(out2, vec![6.0]);
}

#[test]
fn stateful_fixture_accumulates_across_calls() {
    let mut registry = PyBlockRegistry::new();
    let mut instance = registry
        .instantiate(&fixture("accumulator.py"))
        .expect("load accumulator fixture");

    let out1 = instance
        .call(0.0, 1e-5, &[PyInput::Scalar(1.0)], 2)
        .unwrap();
    assert_eq!(out1, vec![1.0, 1.0]);
    let out2 = instance
        .call(0.0, 1e-5, &[PyInput::Scalar(2.0)], 2)
        .unwrap();
    assert_eq!(out2, vec![3.0, 2.0]);
    let out3 = instance
        .call(0.0, 1e-5, &[PyInput::Scalar(4.0)], 2)
        .unwrap();
    assert_eq!(out3, vec![7.0, 3.0]);
}

#[test]
fn two_instances_of_the_same_file_keep_independent_state() {
    let mut registry = PyBlockRegistry::new();
    let mut a = registry
        .instantiate(&fixture("accumulator.py"))
        .expect("load instance a");
    let mut b = registry
        .instantiate(&fixture("accumulator.py"))
        .expect("load instance b");

    a.call(0.0, 1e-5, &[PyInput::Scalar(10.0)], 2).unwrap();
    a.call(0.0, 1e-5, &[PyInput::Scalar(10.0)], 2).unwrap();
    let a_out = a.call(0.0, 1e-5, &[PyInput::Scalar(10.0)], 2).unwrap();
    assert_eq!(a_out, vec![30.0, 3.0]);

    // b has never been called before: its own state must start fresh, not inherit a's --
    // confirms per-instance namespace isolation, not just per-instance `start()` state.
    let b_out = b.call(0.0, 1e-5, &[PyInput::Scalar(1.0)], 2).unwrap();
    assert_eq!(b_out, vec![1.0, 1.0]);
}

#[test]
fn clone_via_deepcopy_produces_independent_state_with_no_author_opt_in() {
    let mut registry = PyBlockRegistry::new();
    let mut original = registry
        .instantiate(&fixture("accumulator.py"))
        .expect("load accumulator fixture");

    original
        .call(0.0, 1e-5, &[PyInput::Scalar(5.0)], 2)
        .unwrap();
    original
        .call(0.0, 1e-5, &[PyInput::Scalar(5.0)], 2)
        .unwrap(); // sum=10, count=2

    let mut cloned = original.try_clone().expect("deepcopy-based clone");

    let original_out = original
        .call(0.0, 1e-5, &[PyInput::Scalar(1.0)], 2)
        .unwrap();
    let cloned_out = cloned
        .call(0.0, 1e-5, &[PyInput::Scalar(100.0)], 2)
        .unwrap();

    assert_eq!(original_out, vec![11.0, 3.0]);
    assert_eq!(cloned_out, vec![110.0, 3.0]);
}

#[test]
fn vector_input_arrives_as_a_real_numpy_array() {
    let mut registry = PyBlockRegistry::new();
    let mut instance = registry
        .instantiate(&fixture("vector_sum.py"))
        .expect("load vector_sum fixture");

    let out = instance
        .call(0.0, 1e-5, &[PyInput::Vector(&[1.0, 2.0, 3.0, 4.0])], 1)
        .unwrap();
    assert_eq!(out, vec![10.0]);
}

#[test]
fn rk4_step_xc_matches_the_closed_form_exponential_decay() {
    // dx/dt = -K*x, x0 = 1.0, K = 2.0 (see decay_xc.py): x(t) = exp(-K*t). Same closed-form
    // check as cscript-ffi's own equivalent test, confirming the Python xc path integrates
    // identically to the C one.
    let mut registry = PyBlockRegistry::new();
    let instance = registry
        .instantiate_xc(&fixture("decay_xc.py"))
        .expect("load decay_xc fixture via the xc contract");

    let k = 2.0_f64;
    let dt = 1e-3;
    let mut xc = vec![1.0_f64];
    let mut t = 0.0_f64;
    for _ in 0..200 {
        xc = instance
            .rk4_step_xc(&xc, t, &[PyInput::Scalar(0.0)], dt)
            .unwrap();
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
fn instantiate_rejects_an_xc_only_file_and_instantiate_xc_rejects_a_plain_one() {
    let plain = fixture("gain.py");
    let mut registry = PyBlockRegistry::new();
    let err = registry
        .instantiate_xc(&plain)
        .expect_err("a plain output()-only file must not satisfy the xc contract");
    assert!(
        matches!(
            err,
            PyBlockError::MissingFunction {
                name: "derivative",
                ..
            }
        ),
        "expected MissingFunction(derivative), got: {err}"
    );

    let xc_only = fixture("decay_xc.py");
    let mut registry2 = PyBlockRegistry::new();
    let err2 = registry2
        .instantiate(&xc_only)
        .expect_err("an xc-only file must not satisfy the plain output() contract");
    assert!(
        matches!(err2, PyBlockError::MissingFunction { name: "output", .. }),
        "expected MissingFunction(output), got: {err2}"
    );
}

#[test]
fn a_python_exception_is_a_clear_error_not_a_panic() {
    let mut registry = PyBlockRegistry::new();
    let mut instance = registry
        .instantiate(&fixture("raises.py"))
        .expect("load raises fixture");

    let err = instance
        .call(0.0, 1e-5, &[PyInput::Scalar(1.0)], 1)
        .expect_err("output() deliberately raises");
    match err {
        PyBlockError::Exception {
            function, message, ..
        } => {
            assert_eq!(function, "output");
            assert!(
                message.contains("deliberate failure"),
                "error should include the Python exception's own message, got: {message}"
            );
        }
        other => panic!("expected Exception, got {other:?}"),
    }
}

#[test]
fn missing_start_function_is_a_clear_error_not_a_panic() {
    let out_dir = std::env::temp_dir().join("pyblock-ffi-test-fixtures");
    std::fs::create_dir_all(&out_dir).expect("create fixture output dir");
    let path = out_dir.join("broken_missing_start.py");
    std::fs::write(&path, "def output(state, t, dt, inputs):\n    return 0.0\n")
        .expect("write broken fixture source");

    let mut registry = PyBlockRegistry::new();
    let err = registry
        .instantiate(&path)
        .expect_err("missing start() must be reported, not panic");
    assert!(
        matches!(err, PyBlockError::MissingFunction { name: "start", .. }),
        "expected MissingFunction(start), got: {err}"
    );
}

#[test]
fn output_is_read_only_and_update_is_the_sole_place_state_advances() {
    let mut registry = PyBlockRegistry::new();
    let mut instance = registry
        .instantiate(&fixture("update_counter.py"))
        .expect("load update_counter fixture");

    // Calling output() any number of times with no update() must never change the result.
    let out1 = instance.call(0.0, 0.0, &[PyInput::Scalar(5.0)], 1).unwrap();
    let out2 = instance.call(0.0, 0.0, &[PyInput::Scalar(5.0)], 1).unwrap();
    assert_eq!(out1, vec![0.0]);
    assert_eq!(out2, vec![0.0]);

    // update() is the only place the accumulator actually advances.
    instance.update(0.0, 0.0, &[PyInput::Scalar(5.0)]).unwrap();
    let out3 = instance.call(0.0, 0.0, &[PyInput::Scalar(5.0)], 1).unwrap();
    assert_eq!(out3, vec![5.0]);

    instance.update(0.0, 0.0, &[PyInput::Scalar(3.0)]).unwrap();
    let out4 = instance.call(0.0, 0.0, &[PyInput::Scalar(5.0)], 1).unwrap();
    assert_eq!(out4, vec![8.0]);
}

#[test]
fn update_is_a_silent_no_op_when_the_py_file_does_not_define_it() {
    // Every existing fixture predates update() -- confirms the new resolution is genuinely
    // optional and doesn't break a .py file that never heard of it.
    let mut registry = PyBlockRegistry::new();
    let mut instance = registry
        .instantiate(&fixture("accumulator.py"))
        .expect("load accumulator fixture");
    instance
        .update(0.0, 0.0, &[PyInput::Scalar(999.0)])
        .unwrap(); // must not error
    let out = instance.call(0.0, 0.0, &[PyInput::Scalar(1.0)], 2).unwrap();
    assert_eq!(out, vec![1.0, 1.0]); // unaffected by the no-op update() call above
}
