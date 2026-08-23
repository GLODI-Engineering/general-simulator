//! `kind=pmsm` through the real CLI device-file parser and CSV output — an end-to-end check
//! that the grammar documented in `main.rs`'s module doc comment actually parses and reaches
//! `dae-runtime`'s block graph, not just the `BlockKind` construction tested directly in
//! `dae-runtime`'s own `pmsm_block.rs`. Reuses the same decoupled-R-L-circuit hand derivation
//! (`vq=0`, `iq(0)=0` keeps `iq`/`omega_m` at zero, leaving `id(t) = (vd/R)*(1 -
//! exp(-t*R/Ld))`). Uses a dummy always-off MOSFET purely so `simulate_transient_with_blocks`'s
//! code path is reached at all (same convention as `tests/cscript.rs`).

use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn write_devices_file(contents: &str) -> PathBuf {
    let out_dir = std::env::temp_dir().join("elspice-pwl-cli-test-fixtures");
    std::fs::create_dir_all(&out_dir).expect("create fixture output dir");
    let path = out_dir.join(format!(
        "pmsm-devices-{}.txt",
        std::process::id() as u64 * 1000 + contents.len() as u64
    ));
    std::fs::write(&path, contents).expect("write devices file");
    path
}

fn run(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_elspice-pwl"))
        .args(args)
        .output()
        .expect("failed to run elspice-pwl binary");
    assert!(
        output.status.success(),
        "elspice-pwl exited with {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout was not valid UTF-8")
}

#[test]
fn pmsm_block_matches_hand_derived_rl_circuit_when_decoupled() {
    let netlist = fixture("cscript_gain.cir"); // V1 a 0 5 / D1 a b mosfetmodel / R1 b 0 1000
    let (r, l, vd) = (2.0, 5e-3, 10.0);
    let tau = l / r;
    let dt = tau / 200.0;
    let t_final = dt * 400.0;

    let devices = write_devices_file(&format!(
        "OFFVAL kind=const value=0\n\
         OFFGATE kind=sig2gate in=OFFVAL\n\
         D1 kind=mosfet r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=1e6 g_on=0 gate=block ctrl=OFFGATE\n\
         VD kind=const value={vd}\n\
         VQ kind=const value=0\n\
         TL kind=const value=0\n\
         M1 kind=pmsm r_s={r} l_d={l} l_q={l} lambda_pm=0.05 pole_pairs=4 inertia=1e-4 \
         friction=0 inputs=VD,VQ,TL\n"
    ));

    let stdout = run(&[
        netlist.to_str().unwrap(),
        "--devices",
        devices.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        &t_final.to_string(),
        "--dt",
        &dt.to_string(),
    ]);

    let lines: Vec<&str> = stdout.lines().collect();
    let header: Vec<&str> = lines[0].split(',').collect();
    for expected in ["M1", "M1_iq", "M1_omega_m", "M1_theta_e"] {
        assert!(
            header.contains(&expected),
            "expected column '{expected}' in header {header:?}"
        );
    }

    let last_row: Vec<&str> = lines.last().unwrap().split(',').collect();
    let t_col = header.iter().position(|h| *h == "t").unwrap();
    let t_last: f64 = last_row[t_col].parse().unwrap();
    let col = |name: &str| -> f64 {
        let idx = header.iter().position(|h| *h == name).unwrap();
        last_row[idx].parse().unwrap()
    };
    let expected_id = (vd / r) * (1.0 - (-t_last * r / l).exp());

    let tol = 1e-6;
    assert!((col("M1") - expected_id).abs() < tol, "id={}", col("M1"));
    assert!(col("M1_iq").abs() < tol, "iq={}", col("M1_iq"));
    assert!(
        col("M1_omega_m").abs() < tol,
        "omega_m={}",
        col("M1_omega_m")
    );
    assert!(
        col("M1_theta_e").abs() < tol,
        "theta_e={}",
        col("M1_theta_e")
    );
}
