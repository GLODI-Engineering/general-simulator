//! What time a CSV row's values belong to.
//!
//! Reported once as "every CSV row's value is one dt behind its own timestamp"
//! (`general-simulator#21`). It is not: a circuit unknown and a signal-domain block that does
//! not read the circuit are both the value at the row's own `t`. What *is* one step behind is a
//! `kind=phys2sig` column, by construction — the block graph is evaluated before the circuit is
//! solved, so a block reading the circuit reads the previous accepted step's solution.
//!
//! The two are easy to confuse because the second looks like the first in any sheet that
//! measures through a probe. These tests pin both halves so neither can drift: the first would
//! silently bias every windowed measurement; the second is a documented deliberate coupling
//! (`book/user-guide/src/reading-output.md`, `BlockKind::Phys2Sig`'s implementation notes) that
//! a well-meaning "fix" to the first could just as silently remove.

use std::path::PathBuf;
use std::process::Command;

fn write_netlist(name: &str, contents: &str) -> PathBuf {
    let out_dir = std::env::temp_dir().join("general-simulator-cli-test-fixtures");
    std::fs::create_dir_all(&out_dir).expect("create fixture output dir");
    let path = out_dir.join(format!("row-time-{name}-{}.cir", std::process::id()));
    std::fs::write(&path, contents).expect("write netlist file");
    path
}

fn run(netlist: &PathBuf, args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_general-simulator"))
        .arg(netlist)
        .args(args)
        .output()
        .expect("failed to run general-simulator binary");
    assert!(
        out.status.success(),
        "run failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("stdout is utf-8")
}

/// `t` plus one named column, as parallel vectors.
fn column(stdout: &str, name: &str) -> (Vec<f64>, Vec<f64>) {
    let lines: Vec<&str> = stdout.lines().collect();
    let header: Vec<&str> = lines[0].split(',').collect();
    let t_idx = header.iter().position(|h| *h == "t").unwrap();
    let idx = header
        .iter()
        .position(|h| *h == name)
        .unwrap_or_else(|| panic!("no column '{name}' in header {:?}", header));
    let mut ts = Vec::new();
    let mut vs = Vec::new();
    for line in &lines[1..] {
        let cols: Vec<&str> = line.split(',').collect();
        ts.push(cols[t_idx].parse().unwrap());
        vs.push(cols[idx].parse().unwrap());
    }
    (ts, vs)
}

const AMPLITUDE: f64 = 325.2691;
const FREQ: f64 = 50.0;

#[test]
fn a_transient_source_column_is_the_value_at_the_rows_own_stamp() {
    let netlist = write_netlist(
        "sin-source",
        "* 50 Hz sine across a resistor\n\
         Vac vac_node 0 SIN(0 325.2691 50 0 0 0)\n\
         R1 vac_node 0 1k\n",
    );
    let stdout = run(
        &netlist,
        &["--mode", "transient", "--tfinal", "0.02", "--dt", "1e-4"],
    );
    let (ts, vs) = column(&stdout, "V(vac_node)");
    assert!(
        ts.len() > 100,
        "expected a full cycle of rows, got {}",
        ts.len()
    );

    // Against the row's own stamp the trace is exact to the solver's own precision; against
    // `t - dt` it is off by the step's whole increment, which is the claim being refuted.
    //
    // Comparing the *worst* row of each hypothesis is the discriminating test: near the
    // sinusoid's own peak a one-step shift is almost invisible (that is where a spot check
    // misleads), so only the maximum over a full cycle separates the two. Near a zero
    // crossing the lagged hypothesis is off by about $A \cdot 2\pi f \Delta t \approx 10$ V.
    let dt = 1e-4;
    let mut worst_at_t = 0.0_f64;
    let mut worst_at_t_minus_dt = 0.0_f64;
    for (t, v) in ts.iter().zip(&vs) {
        let at_t = AMPLITUDE * (2.0 * std::f64::consts::PI * FREQ * t).sin();
        let lagged = AMPLITUDE * (2.0 * std::f64::consts::PI * FREQ * (t - dt)).sin();
        worst_at_t = worst_at_t.max((v - at_t).abs());
        worst_at_t_minus_dt = worst_at_t_minus_dt.max((v - lagged).abs());
    }
    assert!(
        worst_at_t < 1e-9,
        "V(vac_node) should equal A*sin(2*pi*f*t) at the row's own stamp; worst error {worst_at_t}"
    );
    assert!(
        worst_at_t_minus_dt > 1.0,
        "the one-step-lagged expression should be clearly refuted somewhere in the cycle; \
         worst error was only {worst_at_t_minus_dt}"
    );
}

#[test]
fn a_signal_block_column_is_the_value_at_the_rows_own_stamp() {
    // The same sinusoid as a block rather than a source: the block graph's own `t`, not the
    // circuit solve's, so this is a genuinely separate path from the test above.
    let netlist = write_netlist(
        "sin-block",
        "* the same 50 Hz sine as a signal-domain source block\n\
         SIG kind=sinwave va=325.2691 freq=50 v0=0 td=0 theta=0 phase=0\n\
         Vdummy n1 0 1\n\
         R1 n1 0 1k\n",
    );
    let stdout = run(
        &netlist,
        &["--mode", "transient", "--tfinal", "0.02", "--dt", "1e-4"],
    );
    let (ts, vs) = column(&stdout, "SIG");
    let mut worst = 0.0_f64;
    for (t, v) in ts.iter().zip(&vs) {
        worst = worst.max((v - AMPLITUDE * (2.0 * std::f64::consts::PI * FREQ * t).sin()).abs());
    }
    assert!(worst < 1e-9, "SIG lags its own stamp; worst error {worst}");
}

#[test]
fn a_pulse_edge_lands_in_the_first_row_past_the_edge_time() {
    // The shape the original report was read off. `pw=6e-6` means the fall begins at exactly
    // 6 us and takes `tf=1e-9`, so the last high row is the last stamp strictly below 6 us and
    // the first low row is the next one -- not the one after that.
    let netlist = write_netlist(
        "pulse",
        "* a bare gate pulse\n\
         GATE kind=pulsewave v1=0 v2=1 td=0 tr=1e-9 tf=1e-9 pw=6e-6 per=1e-5\n\
         Vdummy n1 0 1\n\
         R1 n1 0 1k\n",
    );
    let stdout = run(
        &netlist,
        &["--mode", "transient", "--tfinal", "7e-6", "--dt", "1e-7"],
    );
    let (ts, vs) = column(&stdout, "GATE");
    let last_high = ts
        .iter()
        .zip(&vs)
        .filter(|(_, v)| **v > 0.5)
        .map(|(t, _)| *t)
        .fold(f64::NEG_INFINITY, f64::max);
    let first_low = ts
        .iter()
        .zip(&vs)
        .filter(|(t, v)| **v < 0.5 && *t > &1e-9)
        .map(|(t, _)| *t)
        .fold(f64::INFINITY, f64::min);
    assert!(
        last_high < 6e-6,
        "the last high row is stamped {last_high}, at or past the 6 us fall"
    );
    assert!(
        first_low - last_high < 1.5e-7,
        "the fall spans {} s of stamps, more than the one step it should",
        first_low - last_high
    );
}

#[test]
fn a_phys2sig_column_is_the_previous_steps_solution() {
    // The one real exception, and the one this file exists to keep from being "fixed" away.
    let netlist = write_netlist(
        "phys2sig",
        "V1 in 0 5\n\
         R1 in out 1k\n\
         R2 out 0 1k\n\
         PROBE1 kind=phys2sig node=out\n",
    );
    let stdout = run(
        &netlist,
        &["--mode", "transient", "--tfinal", "4e-3", "--dt", "1e-3"],
    );
    let (_, vout) = column(&stdout, "V(out)");
    let (_, probe) = column(&stdout, "PROBE1");
    assert!(vout.len() >= 3, "expected at least three rows");
    assert_eq!(
        probe[0], 0.0,
        "the first row has no previous step to sample"
    );
    for i in 1..vout.len() {
        assert!(
            (probe[i] - vout[i - 1]).abs() < 1e-12,
            "row {i}: PROBE1={} should be the previous row's V(out)={}",
            probe[i],
            vout[i - 1]
        );
    }
    // And in its own row it disagrees with the column it reads, which is the thing a user
    // notices and mistakes for a global offset.
    assert!(
        (probe[0] - vout[0]).abs() > 1.0,
        "the first row should visibly differ: PROBE1={}, V(out)={}",
        probe[0],
        vout[0]
    );
}
