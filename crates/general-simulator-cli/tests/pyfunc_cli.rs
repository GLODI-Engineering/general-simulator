//! Runs the actual built `general-simulator` binary against a netlist using `kind=pyfunc` --
//! a genuinely separate contract from `pyblock_cli.rs`'s own `kind=pyblock` test. Same dummy
//! always-off ideal switch pattern those tests use, for the same reason, and the same "copy fixtures
//! into a space-free temp dir" workaround `pyblock_cli.rs` needed on this machine.

use std::path::PathBuf;
use std::process::Command;

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
        "devices-pyfunc-{}.txt",
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
fn pyfunc_reproduces_the_truth_table_through_the_real_cli() {
    let py_path = fixture("gate_pattern.py");
    let devices = write_devices_file(&format!(
        "OFFVAL kind=const value=0\n\
         OFFGATE kind=sig2phys domain=voltage in=OFFVAL\n\
         D1 kind=ideal_switch r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.7 g_on=1 gate=block ctrl=OFFGATE\n\
         PHASE kind=const value=270\n\
         AQ kind=pyfunc path={} function=compute_action_qualifier_180_degree in=PHASE outputs=AQCTLA,AQCTLB\n",
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
    let header: Vec<&str> = lines[0].split(',').collect();
    assert!(
        header.contains(&"AQ") && header.contains(&"AQCTLB"),
        "expected AQ (primary, aliasing AQCTLA) and AQCTLB columns, got: {header:?}"
    );
    let aq_idx = header.iter().position(|h| *h == "AQ").unwrap();
    let aqb_idx = header.iter().position(|h| *h == "AQCTLB").unwrap();
    // phase=270 is in (180,360) -> AQCTLA=1057, AQCTLB=2066.
    for line in &lines[1..] {
        let fields: Vec<&str> = line.split(',').collect();
        let aq: f64 = fields[aq_idx].parse().unwrap();
        let aqb: f64 = fields[aqb_idx].parse().unwrap();
        assert_eq!(aq, 1057.0);
        assert_eq!(aqb, 2066.0);
    }
}
