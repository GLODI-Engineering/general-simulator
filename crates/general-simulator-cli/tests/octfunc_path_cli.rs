//! An `octfunc` block's `path=` must be the file that runs.
//!
//! `octave-ffi` turns `path=` into an `addpath` of the file's *parent directory* and then calls
//! the function *by name*. Octave resolves a name against its whole search path with the current
//! working directory first, so `path=` only ever added one more directory where the file *might*
//! be found. Two directories defining one function name, and the run executed whichever Octave
//! reached first -- completing, exiting 0, and writing a full waveform computed by a file the
//! netlist never mentioned.
//!
//! Measured before the fix, from a directory holding a `dbl2.m` returning the constant 999 while
//! the netlist named a `dbl2.m` in `other/` returning `2*u`:
//!
//! ```text
//! t,V(n1),V(n2),I(V1),VPROBE,G1
//! 0.00001,0.0627905195,0.0006216883,-0.0000621688,0,999
//! ```
//!
//! These tests need `octave-cli` on `PATH`, like every other Octave test in this workspace.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_BIN_EXE_general-simulator"));
    path.set_extension(std::env::consts::EXE_EXTENSION);
    path
}

/// A scratch directory holding a generic RC low-pass, a `dbl2.m` that returns a constant, and an
/// `other/dbl2.m` that doubles its input. Keyed by `case_name` so parallel tests never share one.
fn shadowed_case(case_name: &str, netlist_body: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gsim-octfunc-path-{case_name}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("other")).expect("scratch directory");
    fs::write(
        dir.join("dbl2.m"),
        "function y = dbl2(u)\n  y = 999;\nend\n",
    )
    .unwrap();
    fs::write(
        dir.join("other/dbl2.m"),
        "function y = dbl2(u)\n  y = 2*u;\nend\n",
    )
    .unwrap();
    fs::write(dir.join("rc.cir"), netlist_body).unwrap();
    dir
}

const RC_PREAMBLE: &str = "* Generic RC low-pass; one code block reads the capacitor voltage.\n\
                           V1 n1 0 SIN(0 1 1k)\n\
                           R1 n1 n2 1k\n\
                           C1 n2 0 1u\n\
                           VPROBE kind=phys2sig node=n2\n";

fn run_in(dir: &Path) -> std::process::Output {
    Command::new(binary())
        .current_dir(dir)
        .args([
            "rc.cir",
            "--mode",
            "transient",
            "--tfinal",
            "3e-5",
            "--dt",
            "1e-5",
        ])
        .output()
        .expect("running the solver")
}

#[test]
fn a_function_resolving_to_another_file_is_refused_naming_both() {
    let dir = shadowed_case(
        "shadowed",
        &format!(
            "{RC_PREAMBLE}G1 kind=octfunc path=./other/dbl2.m function=dbl2 in=VPROBE outputs=y\n"
        ),
    );
    let output = run_in(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "a run whose block resolves to a file the netlist never named must be refused; it \
         produced:\n{stdout}"
    );
    assert!(
        !stdout.contains("999"),
        "the shadowing file's own answer reached the output:\n{stdout}"
    );
    // Both paths, because the user has to be able to see which one won.
    assert!(
        stderr.contains("other/dbl2.m"),
        "the error does not name the file the netlist declared: {stderr}"
    );
    assert!(
        stderr.contains("dbl2.m") && stderr.matches("dbl2.m").count() >= 2,
        "the error does not name both files: {stderr}"
    );
}

#[test]
fn a_path_that_does_not_exist_is_refused_even_when_the_name_resolves_elsewhere() {
    // `path=./nope.m` does not exist, and `dbl2` is reachable from the working directory. Before
    // the fix this ran to completion on the wrong file, so a path typo was not an error at all.
    let dir = shadowed_case(
        "missing-path",
        &format!("{RC_PREAMBLE}G1 kind=octfunc path=./nope.m function=dbl2 in=VPROBE outputs=y\n"),
    );
    let output = run_in(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        !output.status.success(),
        "a block whose `path=` does not exist must be refused; it produced:\n{stdout}"
    );
    assert!(
        stderr.contains("nope.m"),
        "the error does not name the path that is missing: {stderr}"
    );
}

#[test]
fn a_function_resolving_to_the_declared_file_still_runs() {
    // The same shadowing layout, but the netlist names the file that actually wins. The check
    // must not refuse a correct sheet.
    let dir = shadowed_case(
        "agreeing",
        &format!("{RC_PREAMBLE}G1 kind=octfunc path=./dbl2.m function=dbl2 in=VPROBE outputs=y\n"),
    );
    let output = run_in(&dir);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "a block naming the file that resolves must run: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains("999"),
        "the declared file's own answer did not reach the output:\n{stdout}"
    );
}
