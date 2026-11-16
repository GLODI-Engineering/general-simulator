//! `--version` and `--help`, and what happens to a flag typed before the netlist.
//!
//! Every argument used to be a netlist path to this CLI, so `--version` came back as
//! `error: reading --version: No such file or directory` with status 1. That is not a
//! cosmetic problem: anything probing the binary to decide whether it works read that as
//! "not a working solver". The VS Code front end's own discovery did exactly that, and
//! rejected every binary it was pointed at.

use std::process::Command;

fn run(args: &[&str]) -> (bool, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_general-simulator"))
        .args(args)
        .output()
        .expect("failed to run general-simulator binary");
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

#[test]
fn version_succeeds_and_names_the_program_not_the_crate() {
    for flag in ["--version", "-V"] {
        let (ok, stdout, stderr) = run(&[flag]);
        assert!(ok, "{flag} must exit 0; stderr: {stderr}");
        // The crate is `general-simulator-cli`; the binary it produces, and the name
        // anything searching for it uses, is `general-simulator`.
        assert!(
            stdout.starts_with("general-simulator "),
            "{flag} printed {stdout:?}"
        );
        assert!(
            !stdout.contains("general-simulator-cli"),
            "{flag} leaked the crate name: {stdout:?}"
        );
        assert_eq!(stdout.lines().count(), 1, "{flag} printed more than a line");
        assert!(stderr.is_empty(), "{flag} wrote to stderr: {stderr:?}");
    }
}

#[test]
fn help_succeeds_on_stdout_and_covers_every_flag_the_parser_accepts() {
    for flag in ["--help", "-h"] {
        let (ok, stdout, stderr) = run(&[flag]);
        assert!(ok, "{flag} must exit 0; stderr: {stderr}");
        assert!(stderr.is_empty(), "{flag} wrote to stderr: {stderr:?}");

        // Asking for help is not a misuse, so it goes to stdout with a passing status --
        // the opposite of `usage()`, which is for arguments that really are wrong.
        assert!(stdout.contains("USAGE"), "{flag} has no synopsis: {stdout}");

        // Every flag the argument loop accepts has to appear, or the help is already
        // drifting from the parser.
        for expected in [
            "--devices",
            "--mode",
            "--tfinal",
            "--dt",
            "--dt-max",
            "--dt-min",
            "--dt-init",
            "--reltol",
            "--abstol",
            "--max-steps",
            "--format",
            "--out",
            "--out-every",
            "--checkpoint-out",
            "--checkpoint-every",
            "--resume",
            "--help",
            "--version",
        ] {
            assert!(stdout.contains(expected), "{flag} omits {expected}");
        }

        // Defaults are the part a reader cannot infer.
        assert!(stdout.contains("[default: dc]"), "no --mode default");
        assert!(stdout.contains("[default: csv]"), "no --format default");
    }
}

#[test]
fn a_flag_before_the_netlist_says_so_instead_of_blaming_a_missing_file() {
    // Previously: `error: reading --tfinal: No such file or directory`, which names the
    // wrong problem -- the flag is in the wrong place and the netlist is absent.
    let (ok, _stdout, stderr) = run(&["--tfinal", "1e-3"]);
    assert!(!ok, "a flag before the netlist must fail");
    assert!(
        stderr.contains("unrecognized argument '--tfinal'"),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("netlist path comes first"),
        "the message must say where the netlist goes: {stderr}"
    );
    assert!(
        !stderr.contains("No such file or directory"),
        "it must not blame a missing file: {stderr}"
    );
}

#[test]
fn no_arguments_still_prints_the_usage_text() {
    let (ok, _stdout, stderr) = run(&[]);
    assert!(!ok);
    assert!(
        stderr.contains("usage: general-simulator <netlist>"),
        "{stderr}"
    );
}
