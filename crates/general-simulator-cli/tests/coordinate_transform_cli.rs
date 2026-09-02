//! `kind=clarke`/`kind=clarkepark` through the real CLI device-file parser and CSV output — an
//! end-to-end check that the grammar documented in `main.rs`'s module doc comment actually
//! parses and reaches `dae-runtime`'s block graph, not just the `BlockKind` construction tested
//! directly in `dae-runtime`'s own `coordinate_transform_block.rs`. Uses a dummy always-off
//! ideal switch purely so `simulate_transient_with_blocks`'s code path is reached at all (same
//! convention as `tests/cscript.rs`) -- it never affects the observed values.

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
        "coord-devices-{}.txt",
        std::process::id() as u64 * 1000 + contents.len() as u64
    ));
    std::fs::write(&path, contents).expect("write devices file");
    path
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
fn clarke_and_clarkepark_blocks_appear_as_named_csv_columns() {
    let netlist = fixture("cscript_gain.cir"); // V1 a 0 5 / D1 a b idealswitchmodel / R1 b 0 1000
    let devices = write_devices_file(
        "OFFVAL kind=const value=0\n\
         OFFGATE kind=sig2phys domain=voltage in=OFFVAL\n\
         D1 kind=ideal_switch r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=1e6 g_on=0 gate=block ctrl=OFFGATE\n\
         A kind=const value=1\n\
         B kind=const value=-0.5\n\
         C kind=const value=-0.5\n\
         THETA kind=const value=0\n\
         CLARKE kind=clarke inputs=A,B,C\n\
         DQ0 kind=clarkepark inputs=A,B,C,THETA\n",
    );

    let stdout = run(&[
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

    let lines: Vec<&str> = stdout.lines().collect();
    let header: Vec<&str> = lines[0].split(',').collect();
    // Primary outputs (CLARKE=alpha, DQ0=d) plus the auto-generated extra-output columns.
    for expected in [
        "CLARKE",
        "CLARKE_beta",
        "CLARKE_zero",
        "DQ0",
        "DQ0_q",
        "DQ0_zero",
    ] {
        assert!(
            header.contains(&expected),
            "expected column '{expected}' in header {header:?}"
        );
    }

    let last_row: Vec<&str> = lines.last().unwrap().split(',').collect();
    let col = |name: &str| -> f64 {
        let idx = header.iter().position(|h| *h == name).unwrap();
        last_row[idx].parse().unwrap()
    };

    let tol = 1e-6;
    assert!((col("CLARKE") - 1.0).abs() < tol, "alpha={}", col("CLARKE"));
    assert!(
        col("CLARKE_beta").abs() < tol,
        "beta={}",
        col("CLARKE_beta")
    );
    assert!(
        col("CLARKE_zero").abs() < tol,
        "zero={}",
        col("CLARKE_zero")
    );
    assert!((col("DQ0") - 1.0).abs() < tol, "d={}", col("DQ0"));
    assert!(col("DQ0_q").abs() < tol, "q={}", col("DQ0_q"));
    assert!(col("DQ0_zero").abs() < tol, "zero={}", col("DQ0_zero"));
}
