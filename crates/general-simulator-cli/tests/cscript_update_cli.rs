//! Runs the actual built `general-simulator` binary against `kind=cscript`/`kind=pyblock`
//! netlists exercising the new, optional `cscript_update`/Python `update()` function -- an
//! end-to-end confirmation (real CLI, real netlist, real block graph) that output() staying
//! read-only and update() being the sole place state advances actually works through the whole
//! stack, not just `cscript-ffi`'s/`pyblock-ffi`'s own lower-level unit tests. No dummy MOSFET
//! needed: the block graph runs whenever a block is declared, MOSFET or not.

use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn copy_to_space_free_dir(name: &str) -> PathBuf {
    let source = fixture(name);
    let out_dir = std::env::temp_dir().join("general-simulator-cli-test-fixtures");
    std::fs::create_dir_all(&out_dir).expect("create fixture output dir");
    let dest = out_dir.join(name);
    std::fs::copy(&source, &dest).expect("copy fixture into a space-free temp dir");
    dest
}

fn compile_c_fixture(name: &str) -> PathBuf {
    let source = fixture(&format!("{name}.c"));
    let out_dir = std::env::temp_dir().join("general-simulator-cli-test-fixtures");
    std::fs::create_dir_all(&out_dir).expect("create fixture output dir");
    let lib_path = out_dir.join(format!("lib{name}.so"));
    let status = Command::new("cc")
        .args(["-shared", "-fPIC", "-O0", "-o"])
        .arg(&lib_path)
        .arg(&source)
        .status()
        .expect("run cc to compile fixture");
    assert!(status.success(), "cc failed to compile fixture {name}");
    lib_path
}

fn write_devices_file(contents: &str) -> PathBuf {
    let out_dir = std::env::temp_dir().join("general-simulator-cli-test-fixtures");
    std::fs::create_dir_all(&out_dir).expect("create fixture output dir");
    let path = out_dir.join(format!(
        "devices-update-{}.txt",
        std::process::id() as u64 * 1000 + contents.len() as u64
    ));
    std::fs::write(&path, contents).expect("write devices file");
    path
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_general-simulator"))
        .args(args)
        .output()
        .expect("failed to run general-simulator binary")
}

#[test]
fn cscript_update_advances_the_accumulator_ahead_of_a_read_only_output() {
    let lib = compile_c_fixture("update_counter");
    let netlist = write_devices_file("V1 a 0 5\nR1 a 0 1000\n");
    let devices = write_devices_file(&format!(
        "SRC kind=const value=2\n\
         CNT kind=cscript lib={} in=SRC\n",
        lib.display()
    ));

    let output = run(&[
        netlist.to_str().unwrap(),
        "--devices",
        devices.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "0.0004",
        "--dt",
        "0.0001",
    ]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    // Each row's own output() sees last step's already-committed sum, not this step's own
    // update() -- 0, 2, 4, 6, not 2, 4, 6, 8 -- confirming update() genuinely runs *after*
    // output() each step, not before/inside it.
    let values: Vec<f64> = lines[1..]
        .iter()
        .map(|l| l.split(',').next_back().unwrap().parse().unwrap())
        .collect();
    assert_eq!(values, vec![0.0, 2.0, 4.0, 6.0], "rows: {lines:?}");
}

#[test]
fn pyblock_update_advances_the_accumulator_ahead_of_a_read_only_output() {
    let py_path = copy_to_space_free_dir("update_counter.py");
    let netlist = write_devices_file("V1 a 0 5\nR1 a 0 1000\n");
    let devices = write_devices_file(&format!(
        "SRC kind=const value=2\n\
         CNT kind=pyblock path={} in=SRC\n",
        py_path.display()
    ));

    let output = run(&[
        netlist.to_str().unwrap(),
        "--devices",
        devices.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "0.0004",
        "--dt",
        "0.0001",
    ]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    let values: Vec<f64> = lines[1..]
        .iter()
        .map(|l| l.split(',').next_back().unwrap().parse().unwrap())
        .collect();
    assert_eq!(values, vec![0.0, 2.0, 4.0, 6.0], "rows: {lines:?}");
}
