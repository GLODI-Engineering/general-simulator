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

/// `--capabilities` says what *this* binary can run, so a front end does not have to find out
/// by failing.
///
/// The question it answers is not cosmetic. `kind=pyblock` and `kind=pyfunc` need a
/// `--features python` build, and **the binary the release workflow publishes is not one** --
/// deliberately, because linking against one specific libpython would make it a worse download.
/// So a user who downloads a release, or who follows the build line in a front end's own
/// setting description, gets an engine that silently excludes the two most-used extensibility
/// blocks. Until now the only way to discover that was to draw a sheet, press Run and read
/// `PythonSupportNotCompiledIn`.
///
/// `--help` cannot answer it: its text says `kind=pyblock and kind=pyfunc need a --features
/// python build of this binary` whatever this binary is, so screen-scraping it tells a caller
/// nothing about the binary in front of them.
#[test]
fn capabilities_says_whether_this_binary_can_run_a_python_block() {
    let (ok, stdout, stderr) = run(&["--capabilities"]);
    assert!(ok, "--capabilities must exit 0; stderr: {stderr}");
    assert!(
        stderr.is_empty(),
        "--capabilities wrote to stderr: {stderr:?}"
    );

    // `key=value`, one per line: greppable from a shell, parseable in four lines of any
    // language, and extensible without breaking a reader that does not know a new key.
    let fields: std::collections::BTreeMap<&str, &str> = stdout
        .lines()
        .filter_map(|line| line.split_once('='))
        .collect();

    assert_eq!(
        fields.get("program"),
        Some(&"general-simulator"),
        "no program name: {stdout}"
    );
    assert!(fields.contains_key("version"), "no version: {stdout}");
    assert_eq!(
        fields.get("python"),
        Some(&if cfg!(feature = "python") {
            "yes"
        } else {
            "no"
        }),
        "`python` does not report this build's own feature set: {stdout}"
    );
    // Octave is a subprocess, so the binary is always *able* to try; whether `octave-cli` is on
    // PATH is a different question and not one a build can answer.
    assert_eq!(
        fields.get("octave"),
        Some(&"subprocess"),
        "`octave` does not say how Octave is reached: {stdout}"
    );
    assert_eq!(
        fields.get("cscript"),
        Some(&"load-only"),
        "`cscript` does not say that this binary never compiles: {stdout}"
    );
}

/// The answer is machine-readable and stays so: every line is `key=value` with no blanks, no
/// prose and no banner. A front end that regexes prose is a front end that breaks on a reword.
#[test]
fn capabilities_is_only_key_equals_value_lines() {
    let (_, stdout, _) = run(&["--capabilities"]);
    assert!(!stdout.trim().is_empty(), "--capabilities printed nothing");
    for line in stdout.lines() {
        let (key, value) = line
            .split_once('=')
            .unwrap_or_else(|| panic!("`{line}` is not key=value"));
        assert!(
            !key.is_empty() && !value.is_empty(),
            "`{line}` has an empty half"
        );
        assert!(
            key.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "`{key}` is not a lowercase key"
        );
    }
}

/// `--help` lists it, because a flag nobody can find is a flag nobody uses.
#[test]
fn help_mentions_capabilities() {
    let (_, stdout, _) = run(&["--help"]);
    assert!(
        stdout.contains("--capabilities"),
        "--help omits it: {stdout}"
    );
}
