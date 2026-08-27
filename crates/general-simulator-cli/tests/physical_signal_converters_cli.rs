//! `kind=probe`/`kind=sig2voltage`/`kind=sig2current` through the real CLI parser — an
//! end-to-end check that the enforced physical/signal-domain converter grammar documented in
//! `main.rs`'s module doc comment actually parses and reaches `dae-runtime`'s block graph, not
//! just the `BlockKind` construction tested directly in `dae-runtime`'s own
//! `probe_block.rs`/`sig2voltage_gate_enforcement.rs`/`sig2voltage_sig2current.rs`.

use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn write_devices_file(contents: &str) -> PathBuf {
    let out_dir = std::env::temp_dir().join("general-simulator-cli-test-fixtures");
    std::fs::create_dir_all(&out_dir).expect("create fixture output dir");
    let path = out_dir.join(format!(
        "converters-devices-{}.txt",
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
fn probe_reads_a_node_voltage_through_the_real_parser() {
    let netlist = fixture("cscript_gain.cir"); // V1 a 0 5 / D1 a b mosfetmodel / R1 b 0 1000
    let devices = write_devices_file(
        "OFFVAL kind=const value=0\n\
         OFFGATE kind=sig2voltage in=OFFVAL\n\
         D1 kind=mosfet r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=1e6 g_on=0 gate=block ctrl=OFFGATE\n\
         VMEAS kind=probe node=b\n",
    );

    let output = run(&[
        netlist.to_str().unwrap(),
        "--devices",
        devices.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "1e-6",
        "--dt",
        "1e-7",
    ]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    let header: Vec<&str> = lines[0].split(',').collect();
    assert!(header.contains(&"VMEAS"), "header={header:?}");

    let last: Vec<&str> = lines.last().unwrap().split(',').collect();
    let vmeas_idx = header.iter().position(|h| *h == "VMEAS").unwrap();
    let v_b_idx = header.iter().position(|h| *h == "V(b)").unwrap();
    let vmeas: f64 = last[vmeas_idx].parse().unwrap();
    let v_b_prev: f64 = {
        // Probe reads the *previous* step -- compare against the second-to-last row's own
        // V(b), not the last row's (which is what a same-step read would wrongly expect).
        let prev: Vec<&str> = lines[lines.len() - 2].split(',').collect();
        prev[v_b_idx].parse().unwrap()
    };
    assert!(
        (vmeas - v_b_prev).abs() < 1e-9,
        "VMEAS={vmeas} should equal the previous step's own V(b)={v_b_prev}"
    );
}

#[test]
fn a_gate_naming_a_raw_block_instead_of_sig2voltage_is_rejected_with_a_clear_error() {
    let netlist = fixture("cscript_gain.cir");
    let devices = write_devices_file(
        "DUTY kind=const value=0.3\n\
         D1 kind=mosfet r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=1e6 g_on=0 \
         gate=block ctrl=DUTY\n",
    );

    let output = run(&[
        netlist.to_str().unwrap(),
        "--devices",
        devices.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "1e-6",
        "--dt",
        "1e-7",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("GateTargetNotSig2Voltage") || stderr.contains("Sig2Voltage"),
        "expected a Sig2Voltage-related error, got: {stderr}"
    );
}

#[test]
fn sig2voltage_wrapper_makes_gate_block_work() {
    let netlist = fixture("cscript_gain.cir");
    let devices = write_devices_file(
        "DUTY kind=const value=0.3\n\
         DUTY_GATE kind=sig2voltage in=DUTY\n\
         D1 kind=mosfet r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=1e6 g_on=0 \
         gate=block ctrl=DUTY_GATE\n",
    );

    let output = run(&[
        netlist.to_str().unwrap(),
        "--devices",
        devices.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "1e-6",
        "--dt",
        "1e-7",
    ]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn sig2voltage_drives_a_voltage_source_via_its_own_literal_value_field() {
    // A dummy always-off MOSFET reaches the block-graph code path; the real action is CMD_V
    // driving V1's own magnitude directly, checked against Ohm's law on a pure resistive
    // divider (V(a) must equal CMD's own commanded value exactly).
    let devices = write_devices_file(
        "OFFVAL kind=const value=0\n\
         OFFGATE kind=sig2voltage in=OFFVAL\n\
         D1 kind=mosfet r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=1e6 g_on=0 gate=block ctrl=OFFGATE\n\
         CMD kind=const value=7\n\
         CMD_V kind=sig2voltage in=CMD\n",
    );
    let netlist_path = write_devices_file("V1 a 0 CMD_V\nD1 a b mosfetmodel\nR1 b 0 1000\n");

    let output = run(&[
        netlist_path.to_str().unwrap(),
        "--devices",
        devices.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "1e-6",
        "--dt",
        "1e-7",
    ]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    let header: Vec<&str> = lines[0].split(',').collect();
    let v_a_idx = header.iter().position(|h| *h == "V(a)").unwrap();
    let last: Vec<&str> = lines.last().unwrap().split(',').collect();
    let v_a: f64 = last[v_a_idx].parse().unwrap();
    assert!(
        (v_a - 7.0).abs() < 1e-6,
        "V(a)={v_a}, expected exactly 7.0 (V1 driven by CMD_V=CMD=7)"
    );
}
