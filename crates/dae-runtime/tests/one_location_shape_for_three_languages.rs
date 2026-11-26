//! `DaeError::code_location` answers one question the same way whatever language failed.
//!
//! The defect this closes is not a missing field; it is that a front end needed a different
//! reader per language. Octave put the line and column inside an English sentence, Python
//! reported neither at runtime, and C has no runtime error channel at all. A VS Code
//! `Diagnostic` wants `(uri, line, column, message)`, and it was obtainable for one of the
//! three.
//!
//! The per-language structs are checked where they are produced, against real `octave-cli` and
//! real CPython (`octave-ffi`'s and `pyblock-ffi`'s own suites). This file checks the *mapping*,
//! which is the part a front end depends on, and the one claim that cannot be checked anywhere
//! else: that a C block's failure reports no location rather than an invented one.

use std::path::PathBuf;

use dae_runtime::DaeError;

#[test]
fn an_octave_runtime_error_maps_file_line_and_column() {
    let error = DaeError::Octave(octave_ffi::OctaveError::Runtime {
        message: "'nosuchvariable' undefined near line 3, column 13".to_string(),
        location: Some(octave_ffi::OctaveLocation {
            file: Some("/project/ctrl.m".to_string()),
            line: Some(3),
            column: Some(13),
        }),
    });
    let location = error
        .code_location()
        .expect("an Octave error has a location");
    assert_eq!(location.file, Some(PathBuf::from("/project/ctrl.m")));
    assert_eq!(location.line, Some(3));
    assert_eq!(location.column, Some(13));
}

#[test]
fn an_octave_error_this_crate_raised_itself_has_no_location() {
    // A desynchronized call is not a failure *in the user's code*, so there is no line in a
    // `.m` file to point at and inventing one would be worse than `None`.
    let error = DaeError::Octave(octave_ffi::OctaveError::Runtime {
        message: "expected 1 output line(s) from `f`, got 2".to_string(),
        location: None,
    });
    assert_eq!(error.code_location(), None);
}

#[test]
fn a_missing_octave_cli_has_no_location_either() {
    let error = DaeError::Octave(octave_ffi::OctaveError::NotFound {
        message: "No such file or directory (os error 2)".to_string(),
    });
    assert_eq!(error.code_location(), None);
}

/// Every C-block failure, and the claim that matters: **none of them has a location, by
/// construction.** `cscript_output` returns `void`, so a C block has no channel to report a
/// runtime failure through, let alone a position. A load failure names the library and the
/// block, which is the whole of what is knowable.
///
/// This is the test that keeps someone from "improving" the mapping later by pointing a
/// diagnostic at line 1 of a `.c` file nobody read.
#[test]
fn no_c_block_failure_claims_a_location() {
    let failures = [
        DaeError::CScript(cscript_ffi::CScriptError::Load {
            path: PathBuf::from("./ctrl.so"),
            message: "dlopen failed".to_string(),
        }),
        DaeError::CScript(cscript_ffi::CScriptError::MissingSymbol {
            path: PathBuf::from("./ctrl.so"),
            symbol: "cscript_output",
            message: "not found".to_string(),
        }),
        DaeError::CScriptArityMismatch {
            block_name: "G1".to_string(),
            declared_inputs: Some(1),
            declared_outputs: Some(1),
            netlist_inputs: 1,
            netlist_outputs: 2,
        },
        DaeError::CScriptRequiresCloneForAdaptiveStep {
            block_name: "G1".to_string(),
        },
    ];
    for failure in failures {
        assert_eq!(
            failure.code_location(),
            None,
            "a C block's failure must not claim a location: {failure:?}"
        );
    }
}

/// Nor does a failure that has nothing to do with a code block.
#[test]
fn a_circuit_level_failure_has_no_code_location() {
    assert_eq!(DaeError::StopRequested.code_location(), None);
    assert_eq!(
        DaeError::AlgebraicLoop {
            cycle: vec!["A".to_string(), "A".to_string()]
        }
        .code_location(),
        None
    );
}

#[cfg(feature = "python")]
#[test]
fn a_python_exception_maps_file_line_and_the_frame_name() {
    let error = DaeError::PyBlock(pyblock_ffi::PyBlockError::Exception {
        path: PathBuf::from("/project/ctrl.py"),
        function: "output",
        message: "NameError: name 'undefined_name' is not defined".to_string(),
        location: Some(pyblock_ffi::PyLocation {
            file: Some("/project/ctrl.py".to_string()),
            line: Some(14),
            function: Some("output".to_string()),
        }),
    });
    let location = error
        .code_location()
        .expect("a Python error has a location");
    assert_eq!(location.file, Some(PathBuf::from("/project/ctrl.py")));
    assert_eq!(location.line, Some(14));
    assert_eq!(location.function.as_deref(), Some("output"));
    // CPython carries no column for a traceback frame, and a `SyntaxError`'s `offset` is a byte
    // offset rather than a column a reader would recognise, so this is left unset rather than
    // guessed at.
    assert_eq!(location.column, None);
}

#[cfg(feature = "python")]
#[test]
fn a_python_syntax_error_maps_too() {
    let error = DaeError::PyBlock(pyblock_ffi::PyBlockError::Load {
        path: PathBuf::from("/project/ctrl.py"),
        message: "SyntaxError: expected ':'".to_string(),
        location: Some(pyblock_ffi::PyLocation {
            file: Some("/project/ctrl.py".to_string()),
            line: Some(3),
            function: None,
        }),
    });
    let location = error.code_location().expect("a SyntaxError has a location");
    assert_eq!(location.line, Some(3));
}
