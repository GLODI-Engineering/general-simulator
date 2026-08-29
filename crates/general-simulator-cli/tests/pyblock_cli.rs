//! Runs the actual built `general-simulator` binary against netlists using `kind=pyblock`,
//! against the `.py` fixture in `tests/fixtures/` (no compile step needed, unlike `cscript`'s
//! `.c` fixtures -- an end-to-end check through the real CLI parser and `dae-runtime`'s block
//! graph, not just `pyblock-ffi`'s own lower-level unit tests. Same dummy always-off ideal switch
//! pattern `cscript.rs`'s own tests use, for the same reason.

use std::path::PathBuf;
use std::process::Command;

// Copied into a temp dir (never the crate's own source tree, which on this machine lives under
// a path containing a space -- "Github Works/..." -- breaking the netlist grammar's own
// whitespace-delimited field parser) before being referenced from netlist text.
fn fixture(name: &str) -> PathBuf {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let out_dir = std::env::temp_dir().join("general-simulator-cli-test-fixtures");
    std::fs::create_dir_all(&out_dir).expect("create fixture output dir");
    let dest = out_dir.join(name);
    std::fs::copy(&source, &dest).expect("copy fixture into a space-free temp dir");
    dest
}

fn write_devices_file(contents: &str) -> PathBuf {
    let out_dir = std::env::temp_dir().join("general-simulator-cli-test-fixtures");
    std::fs::create_dir_all(&out_dir).expect("create fixture output dir");
    let path = out_dir.join(format!(
        "devices-pyblock-{}.txt",
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
fn pyblock_gain_reproduces_a_hand_known_result_every_step() {
    let py_path = fixture("pyblock_gain.py");
    let devices = write_devices_file(&format!(
        "OFFVAL kind=const value=0\n\
         OFFGATE kind=sig2voltage in=OFFVAL\n\
         D1 kind=ideal_switch r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.7 g_on=1 gate=block ctrl=OFFGATE\n\
         SRC kind=const value=5\n\
         PG kind=pyblock path={} in=SRC\n",
        py_path.display()
    ));
    let netlist = write_devices_file("V1 a 0 5\nD1 a b idealswitchmodel\nR1 b 0 1000\n");

    let output = run(&[
        netlist.to_str().unwrap(),
        "--devices",
        devices.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "0.001",
        "--dt",
        "0.0002",
    ]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(lines[0].ends_with(",PG"), "header: {}", lines[0]);
    for line in &lines[1..] {
        let pg: f64 = line.split(',').next_back().unwrap().parse().unwrap();
        assert!((pg - 10.0).abs() < 1e-12, "expected PG=10 (5*2), got {pg}");
    }
}
