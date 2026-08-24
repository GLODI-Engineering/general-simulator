//! Runs the actual built `general-simulator` binary as a subprocess (the standard way to test a CLI)
//! against fixture netlists that are the *exact same circuits* already hand-verified in
//! `dae-runtime`'s own tests — so the expected output here isn't a fresh hand derivation, it's
//! a cross-check that the CLI's argument parsing, device-file parsing, and CSV formatting
//! correctly reach the same, already-trusted numerical result.

use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn run(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_general-simulator"))
        .args(args)
        .output()
        .expect("failed to run general-simulator binary");
    assert!(
        output.status.success(),
        "general-simulator exited with {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout was not valid UTF-8")
}

#[test]
fn dc_mode_reproduces_the_hand_verified_two_diode_operating_point() {
    let netlist = fixture("two_diode.cir");
    let devices = fixture("two_diode_devices.txt");
    let stdout = run(&[
        netlist.to_str().unwrap(),
        "--devices",
        devices.to_str().unwrap(),
        "--mode",
        "dc",
    ]);

    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "t,V(e1),V(e2),I(V1)");
    let fields: Vec<&str> = lines[1].split(',').collect();
    let v_e2: f64 = fields[2].parse().unwrap();
    assert!(
        (v_e2 - 7.0 / 3.0).abs() < 1e-9,
        "V(e2) = {v_e2}, expected 7/3"
    );
}

#[test]
fn transient_mode_reproduces_the_hand_verified_rc_through_diode_curve() {
    let netlist = fixture("rc_diode.cir");
    let devices = fixture("rc_diode_devices.txt");
    let stdout = run(&[
        netlist.to_str().unwrap(),
        "--devices",
        devices.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "2.0",
        "--dt",
        "0.001",
    ]);

    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines[0], "t,V(a),V(b),V(c),I(V1)");

    // Row 2000 is t=2.0 (1-indexed data rows after the header, 1e-3 step size).
    let fields: Vec<&str> = lines[2000].split(',').collect();
    let t: f64 = fields[0].parse().unwrap();
    let vc: f64 = fields[3].parse().unwrap();
    assert!((t - 2.0).abs() < 1e-9);
    let expected = 4.3 * (1.0 - (-2.0_f64 / 2.0).exp());
    assert!(
        (vc - expected).abs() < 1e-5,
        "V(c) at t=2 = {vc}, expected {expected}"
    );
}
