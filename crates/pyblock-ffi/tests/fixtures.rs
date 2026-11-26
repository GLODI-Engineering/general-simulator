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

#[test]
fn next_sample_hit_reports_the_py_file_own_requested_interval() {
    let mut registry = PyBlockRegistry::new();
    let instance = registry
        .instantiate(&fixture("variable_sample_time.py"))
        .expect("load variable_sample_time fixture");

    assert_eq!(instance.next_sample_hit(0.0, 0.0, &[]).unwrap(), 1.0);
    assert_eq!(instance.next_sample_hit(0.0, 0.0, &[]).unwrap(), 2.0);
    assert_eq!(instance.next_sample_hit(0.0, 0.0, &[]).unwrap(), 4.0);
    assert_eq!(instance.next_sample_hit(0.0, 0.0, &[]).unwrap(), 8.0);
}

#[test]
fn supports_next_sample_hit_is_false_for_every_pre_existing_fixture() {
    let mut registry = PyBlockRegistry::new();
    let instance = registry
        .instantiate(&fixture("accumulator.py"))
        .expect("load fixture");
    assert!(!instance.supports_next_sample_hit());
}

#[test]
fn supports_next_sample_hit_is_true_for_the_variable_sample_time_fixture() {
    let mut registry = PyBlockRegistry::new();
    let instance = registry
        .instantiate(&fixture("variable_sample_time.py"))
        .expect("load fixture");
    assert!(instance.supports_next_sample_hit());
}

/// A Python block raising at line N reports line N, and its own file.
///
/// This is the case the whole location story exists for, and the one that was lost on purpose:
/// `py_traceback` returned `err.to_string()`, which for a `NameError` is one sentence with no
/// frames at all. The information was right there -- `exec_fresh_module` already passes the
/// user's real path as the code object's filename, so every frame reads
/// `File "<the user's own file>", line N, in output` -- and was simply never formatted.
///
/// A Python *runtime* error is also the single most common failure anyone writing a controller
/// block will hit, so "no line, ever" was the worst of the three languages at the thing a front
/// end needs most.
#[test]
fn a_runtime_exception_reports_the_line_it_was_raised_on() {
    let mut registry = PyBlockRegistry::new();
    let mut instance = registry
        .instantiate(&fixture("raises_at_line_14.py"))
        .expect("the fixture loads; it only raises when called");
    let error = instance
        .call(0.0, 1e-5, &[PyInput::Scalar(1.0)], 1)
        .expect_err("the fixture references an undefined name");
    let location = match &error {
        PyBlockError::Exception { location, .. } => location
            .clone()
            .expect("an exception raised inside the user's own file has a location"),
        other => panic!("expected an Exception, got {other:?}"),
    };
    assert_eq!(
        location.line,
        Some(14),
        "the fixture raises on line 14: {location:?}"
    );
    assert!(
        location
            .file
            .as_deref()
            .is_some_and(|f| f.ends_with("raises_at_line_14.py")),
        "the location does not name the block's own file: {location:?}"
    );
    assert_eq!(
        location.function.as_deref(),
        Some("output"),
        "the innermost frame is the block's `output`: {location:?}"
    );
}

/// The message carries the whole traceback, not just the exception's own one-line `str`.
///
/// Both halves are wanted and for different readers: a front end routes the *fields* to a line,
/// and a person reading the Output panel wants the frames. `err.to_string()` gave neither.
#[test]
fn a_runtime_exception_carries_its_traceback_text() {
    let mut registry = PyBlockRegistry::new();
    let mut instance = registry
        .instantiate(&fixture("raises_at_line_14.py"))
        .expect("load the raising fixture");
    let error = instance
        .call(0.0, 1e-5, &[PyInput::Scalar(1.0)], 1)
        .expect_err("the fixture raises");
    let rendered = error.to_string();
    assert!(
        rendered.contains("NameError") && rendered.contains("undefined_name"),
        "the exception itself is gone from the message: {rendered}"
    );
    assert!(
        rendered.contains("line 14"),
        "the traceback frames are gone from the message: {rendered}"
    );
}

/// A syntax error reports its line too, from CPython's own `SyntaxError` attributes rather than
/// from a traceback -- there is no traceback into a file that did not compile.
#[test]
fn a_syntax_error_reports_the_line_it_is_on() {
    let mut registry = PyBlockRegistry::new();
    let error = registry
        .instantiate(&fixture("syntax_error_at_line_3.py"))
        .expect_err("the fixture does not compile");
    let location = match &error {
        PyBlockError::Load { location, .. } => {
            location.clone().expect("a SyntaxError knows its own line")
        }
        other => panic!("expected a Load error, got {other:?}"),
    };
    assert_eq!(location.line, Some(3), "{location:?}");
    assert!(
        location
            .file
            .as_deref()
            .is_some_and(|f| f.ends_with("syntax_error_at_line_3.py")),
        "{location:?}"
    );
}
/// A block importing a module in its own directory works, with no `PYTHONPATH` set.
///
/// `python ctrl.py` puts the script's directory on `sys.path`; this crate never did, so the one
/// thing anyone does after writing a second block -- factor the shared part into a file beside
/// it -- failed with `ModuleNotFoundError`. The directory is derivable from the block's own
/// `path=`, so asking the user to restate it in a setting was asking them to repeat what the
/// netlist already carries.
#[test]
fn a_block_imports_the_module_beside_it() {
    let mut registry = PyBlockRegistry::new();
    let mut instance = registry
        .instantiate(&fixture("sibling_a/ctrl_a.py"))
        .expect("a block importing its own sibling must load");
    let out = instance
        .call(0.0, 1e-5, &[PyInput::Scalar(4.0)], 1)
        .unwrap();
    assert_eq!(out, vec![8.0], "helper.K in sibling_a is 2.0");
}

/// Two blocks in two directories each import their own sibling in one run. `PYTHONPATH` is
/// process-wide, so the workaround this replaces could not express this without the user
/// enumerating both directories by hand.
#[test]
fn two_blocks_in_two_directories_each_import_their_own_sibling() {
    let mut registry = PyBlockRegistry::new();
    let mut a = registry
        .instantiate(&fixture("sibling_a/ctrl_a.py"))
        .expect("load sibling_a");
    let mut b = registry
        .instantiate(&fixture("sibling_b/ctrl_b.py"))
        .expect("load sibling_b");
    let out_a = a.call(0.0, 1e-5, &[PyInput::Scalar(1.0)], 1).unwrap();
    let out_b = b.call(0.0, 1e-5, &[PyInput::Scalar(1.0)], 1).unwrap();
    assert_eq!(out_a, vec![2.0], "sibling_a's own helper.K is 2.0");
    assert_eq!(out_b, vec![10.0], "sibling_b's own helper_b.K is 10.0");
}

/// Two blocks in two directories whose siblings share a *module name* cannot both import their
/// own: `sys.modules` is process-wide, so the second `import helper` returns the first one's.
/// That would be a silent wrong answer -- a block computing with another block's constants --
/// so it is refused, naming both files.
#[test]
fn a_sibling_shadowed_by_another_blocks_sibling_is_refused_naming_both() {
    let mut registry = PyBlockRegistry::new();
    registry
        .instantiate(&fixture("sibling_a/ctrl_a.py"))
        .expect("load sibling_a first, so its `helper` is the cached one");
    let error = registry
        .instantiate(&fixture("sibling_c/ctrl_c.py"))
        .expect_err("a block whose `helper` resolves to another block's `helper` must be refused");
    let rendered = error.to_string();
    assert!(
        rendered.contains("helper"),
        "the error does not name the module: {rendered}"
    );
    assert!(
        rendered.contains("sibling_c") && rendered.contains("sibling_a"),
        "the error does not name both files: {rendered}"
    );
}

/// A file named after a stdlib module does not shadow it. This is why the block's directory is
/// *appended* to `sys.path` rather than prepended: the block's own file is not a program the
/// user launched, it is a file the engine was pointed at, so making a sibling importable must
/// not cost an installed package.
#[test]
fn a_sibling_named_after_a_stdlib_module_does_not_shadow_it() {
    let mut registry = PyBlockRegistry::new();
    let mut instance = registry
        .instantiate(&fixture("shadows_stdlib/ctrl_json.py"))
        .expect("load the block beside a json.py");
    // The fixture asserts `json.dumps` works, which only the real stdlib module has.
    let out = instance
        .call(0.0, 1e-5, &[PyInput::Scalar(2.0)], 1)
        .unwrap();
    assert_eq!(out, vec![6.0]);
}
