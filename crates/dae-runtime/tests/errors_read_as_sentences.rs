//! Every `DaeError` a user can reach reads as a sentence, not as a Rust `Debug` dump.
//!
//! The defect this guards against is the one a reader cannot work around: a `Debug` derive
//! reaching stderr shows a struct literal (`CScript(Load { path: "./gain.so", message:
//! "dlopen failed" })`) where a sentence was written and never used. The check is crude on
//! purpose -- a rendered error containing `{`, `}` or ` { ` is a `Debug` derive leaking, and no
//! sentence this crate writes needs a brace.
//!
//! Exhaustiveness is enforced by the compiler, not by this file: `Display for DaeError` matches
//! every variant by name with no wildcard arm, so a new variant cannot be added without
//! writing its sentence. This file checks that the sentences are *sentences*, and pins the two
//! the code-block path depends on most.

use std::path::PathBuf;

use dae_runtime::DaeError;

/// One instance of every variant that does not need a foreign error value to construct, plus
/// the three FFI-wrapping ones, which are the whole reason this issue exists.
fn representative_errors() -> Vec<DaeError> {
    vec![
        DaeError::Parse("unterminated .subckt".to_string()),
        DaeError::Io("broken pipe".to_string()),
        DaeError::StopRequested,
        DaeError::UnknownDiodeInput("D1".to_string()),
        DaeError::UnknownBlockInput("CTRL".to_string()),
        DaeError::AlgebraicLoop {
            cycle: vec!["A".to_string(), "B".to_string(), "A".to_string()],
        },
        DaeError::CScript(cscript_ffi::CScriptError::Load {
            path: PathBuf::from("./gain.so"),
            message: "dlopen failed".to_string(),
        }),
        DaeError::CScript(cscript_ffi::CScriptError::MissingSymbol {
            path: PathBuf::from("./gain.so"),
            symbol: "cscript_output",
            message: "not found".to_string(),
        }),
        DaeError::CScriptRequiresCloneForAdaptiveStep {
            block_name: "G1".to_string(),
        },
        DaeError::CScriptRequiresNextSampleHitForVariableSampleTime {
            block_name: "G1".to_string(),
        },
        DaeError::CScriptArityMismatch {
            block_name: "G1".to_string(),
            declared_inputs: Some(1),
            declared_outputs: Some(1),
            netlist_inputs: 1,
            netlist_outputs: 2,
        },
        DaeError::PythonSupportNotCompiledIn {
            block_name: "G1".to_string(),
        },
        DaeError::Octave(octave_ffi::OctaveError::NotFound {
            message: "No such file or directory (os error 2)".to_string(),
        }),
        DaeError::Octave(octave_ffi::OctaveError::Runtime {
            message: "'nosuchvariable' undefined near line 3, column 13".to_string(),
            location: Some(octave_ffi::OctaveLocation {
                file: Some("/project/ctrl.m".to_string()),
                line: Some(3),
                column: Some(13),
            }),
        }),
        DaeError::OctBlockMissingRequiredFile {
            block_name: "G1".to_string(),
            expected_path: PathBuf::from("./ctl/ctl_output.m"),
        },
        DaeError::OctBlockRequiresNextSampleHitForVariableSampleTime {
            block_name: "G1".to_string(),
        },
        DaeError::OctBlockDoesNotSupportAdaptiveStep {
            block_name: "G1".to_string(),
        },
        DaeError::AdaptiveStepStalled {
            steps_taken: 100,
            t_reached: 1e-6,
            t_final: 1e-3,
        },
        DaeError::GateTargetNotSig2Voltage {
            gate: "GD".to_string(),
            block: "PWM".to_string(),
            found_kind: "pid",
        },
        DaeError::SourceNotSig2PhysicalConverter {
            source: "V1".to_string(),
            block: "CTRL".to_string(),
            expected_kind: "sig2phys",
            found_kind: "pid",
        },
        DaeError::Sig2PhysUsedAsCircuitNode {
            block: "VDRV".to_string(),
            element: "R1".to_string(),
            node: "VDRV".to_string(),
        },
        DaeError::DuplicateBlockName("G1".to_string()),
        DaeError::WaveCaptureTooShort {
            block: "SRC".to_string(),
            source: "capture.csv".to_string(),
            covered: 1.999e-3,
            t_final: 3e-3,
        },
        DaeError::CheckpointFormat("unsupported version 9".to_string()),
        DaeError::CheckpointIo {
            path: PathBuf::from("run.ckpt"),
            message: "No such file or directory (os error 2)".to_string(),
        },
        DaeError::CheckpointDeckMismatch {
            unknowns_in_checkpoint: vec!["V(n1)".to_string()],
            unknowns_in_deck: vec!["V(n2)".to_string()],
        },
        DaeError::CheckpointUnsupportedBlock {
            block: "G1".to_string(),
            kind: "octblock",
        },
        DaeError::BlockInitialConditionLength {
            block: "G1".to_string(),
            expected: 2,
            got: 3,
        },
        DaeError::VectorSignalNotSupported {
            block: "G1".to_string(),
        },
        DaeError::VectorSignalSizeMismatch {
            block: "G1".to_string(),
            expected: 3,
            got: 2,
        },
    ]
}

#[test]
fn no_error_renders_as_a_debug_dump() {
    for error in representative_errors() {
        let rendered = error.to_string();
        assert!(
            !rendered.contains('{') && !rendered.contains('}'),
            "`{rendered}` reads as a Rust Debug dump, not a sentence (from {error:?})"
        );
        assert!(
            !rendered.is_empty(),
            "a variant rendered as the empty string: {error:?}"
        );
        // A `Debug` derive always leads with the variant's own CamelCase name followed by `(`
        // or ` {`; a sentence does not.
        assert!(
            !rendered.starts_with("CScript(")
                && !rendered.starts_with("Octave(")
                && !rendered.starts_with("PyBlock("),
            "`{rendered}` still leads with a wrapped variant name"
        );
    }
}

#[test]
fn a_python_block_on_a_binary_without_python_is_told_how_to_get_one() {
    let rendered = DaeError::PythonSupportNotCompiledIn {
        block_name: "G1".to_string(),
    }
    .to_string();
    assert!(
        rendered.contains("G1"),
        "the message does not name the block: {rendered}"
    );
    assert!(
        rendered.contains("--features python"),
        "the message does not name the build flag that fixes it: {rendered}"
    );
}

#[test]
fn a_missing_octave_says_to_install_octave() {
    let rendered = DaeError::Octave(octave_ffi::OctaveError::NotFound {
        message: "No such file or directory (os error 2)".to_string(),
    })
    .to_string();
    assert!(
        rendered.contains("octave-cli"),
        "the message does not name the program that is missing: {rendered}"
    );
    assert!(
        rendered.contains("PATH"),
        "the message does not say where it was looked for: {rendered}"
    );
}

#[test]
fn dae_error_is_a_std_error() {
    fn assert_error<E: std::error::Error>(_: &E) {}
    assert_error(&DaeError::StopRequested);
}
