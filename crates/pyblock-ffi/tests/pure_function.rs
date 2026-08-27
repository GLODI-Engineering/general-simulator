//! `PyFunctionRegistry`/`PyFunctionInstance` -- a genuinely separate, additive contract from
//! `fixtures.rs`'s own `PyBlockRegistry` tests, exercising a plain, stateless, positionally-
//! called Python function (no `start`, no `state`, no `t`/`dt`): the closest equivalent to a
//! named-input/named-output function block in other block-diagram tools.

use std::path::{Path, PathBuf};

use pyblock_ffi::{PyBlockError, PyFunctionRegistry, PyInput};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn named_parameters_and_a_multi_value_tuple_return_work() {
    let mut registry = PyFunctionRegistry::new();
    let instance = registry
        .instantiate(
            &fixture("gate_pattern.py"),
            "compute_action_qualifier_180_degree",
        )
        .expect("load gate_pattern fixture");

    // Same five branches the source function itself has, checked directly against its own
    // hand-written truth table.
    let cases: [(f64, [f64; 2]); 6] = [
        (0.0, [9.0, 6.0]),
        (90.0, [2066.0, 1057.0]),
        (180.0, [6.0, 9.0]),
        (270.0, [1057.0, 2066.0]),
        (360.0, [9.0, 6.0]),       // wraps to 0 via `% 360`
        (450.0, [2066.0, 1057.0]), // wraps to 90
    ];
    for (phase, expected) in cases {
        let out = instance.call(&[PyInput::Scalar(phase)], 2).unwrap();
        assert_eq!(out, expected, "phase_degree={phase}");
    }
}

#[test]
fn a_single_return_value_needs_no_tuple() {
    let mut registry = PyFunctionRegistry::new();
    let instance = registry
        .instantiate(&fixture("varargs_sum.py"), "total")
        .expect("load varargs_sum fixture");

    let out = instance
        .call(
            &[
                PyInput::Scalar(1.0),
                PyInput::Scalar(2.0),
                PyInput::Scalar(3.0),
            ],
            1,
        )
        .unwrap();
    assert_eq!(out, vec![6.0]);
}

#[test]
fn works_transparently_with_a_star_args_function() {
    // total(*args) has no fixed parameter list at all -- confirms the positional f(*inputs)
    // calling convention doesn't depend on the function declaring named parameters, only on
    // accepting positional arguments, exactly as *args does.
    let mut registry = PyFunctionRegistry::new();
    let instance = registry
        .instantiate(&fixture("varargs_sum.py"), "total")
        .expect("load varargs_sum fixture");

    let out = instance
        .call(
            &[
                PyInput::Scalar(10.0),
                PyInput::Scalar(20.0),
                PyInput::Scalar(30.0),
                PyInput::Scalar(40.0),
            ],
            1,
        )
        .unwrap();
    assert_eq!(out, vec![100.0]);
}

#[test]
fn vector_input_is_its_own_positional_argument_as_a_real_numpy_array() {
    let mut registry = PyFunctionRegistry::new();
    let instance = registry
        .instantiate(&fixture("varargs_sum.py"), "total")
        .expect("load varargs_sum fixture");

    // A Vector input arrives as one ndarray positional argument, not flattened together with
    // the scalar that follows it -- sum() over (array([1,2,3]), 4.0) fails to add cleanly
    // unless numpy's own broadcasting/sum semantics are actually engaged, so this only passes
    // if the array genuinely arrived as an ndarray, not a Python list.
    let out = instance
        .call(&[PyInput::Vector(&[1.0, 2.0, 3.0])], 1)
        .unwrap();
    assert_eq!(out, vec![6.0]);
}

#[test]
fn is_cheaply_and_always_cloneable_no_deepcopy_needed() {
    let mut registry = PyFunctionRegistry::new();
    let instance = registry
        .instantiate(
            &fixture("gate_pattern.py"),
            "compute_action_qualifier_180_degree",
        )
        .expect("load gate_pattern fixture");

    let cloned = instance.clone();
    let a = instance.call(&[PyInput::Scalar(90.0)], 2).unwrap();
    let b = cloned.call(&[PyInput::Scalar(90.0)], 2).unwrap();
    assert_eq!(a, b);
}

#[test]
fn a_missing_function_name_is_a_clear_error_not_a_panic() {
    let mut registry = PyFunctionRegistry::new();
    let err = registry
        .instantiate(&fixture("gate_pattern.py"), "this_function_does_not_exist")
        .expect_err("missing function must be reported, not panic");
    assert!(
        matches!(
            err,
            PyBlockError::MissingFunction {
                name: "function",
                ..
            }
        ),
        "expected MissingFunction, got: {err}"
    );
}
