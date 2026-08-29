//! `kind=pwc`/`kind=pwl` (with `repeat=true`) and `kind=sinwave`/`kind=pulsewave`/
//! `kind=expwave`/`kind=sffmwave` through the real CLI device-file parser. The `...wave` naming
//! is itself under test here, not incidental: `kind=sin`/`kind=exp` already exist as
//! waveform-arithmetic *functions* (`sin(x)`/`exp(x)` of an input signal, via the `MathFn1`
//! fallback dispatch) — `sinwave_and_sin_coexist_in_one_file_without_collision` locks in that
//! the source block and the math function are never confused with each other by the parser.

use std::path::PathBuf;
use std::process::Command;

fn write_netlist(contents: &str) -> PathBuf {
    let out_dir = std::env::temp_dir().join("general-simulator-cli-test-fixtures");
    std::fs::create_dir_all(&out_dir).expect("create fixture output dir");
    let path = out_dir.join(format!(
        "signal-domain-sources-{}.cir",
        std::process::id() as u64 * 1000 + contents.len() as u64
    ));
    std::fs::write(&path, contents).expect("write netlist file");
    path
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_general-simulator"))
        .args(args)
        .output()
        .expect("failed to run general-simulator binary")
}

/// A permanently-off ideal switch on its own isolated (grounded-through-1e9-ohm) node pair, wired to
/// nothing else in the real circuit -- purely so `--mode transient` selects the block-graph-
/// enabled solver path (gated on at least one declared ideal switch; none of these netlists have a
/// real one). See `general-simulator-cli`'s own `cscript.rs` test for the general convention, and
/// `regulators.cir` (`internal-archive` repo) for this exact isolated-dummy variant.
const DUMMY_IDEAL_SWITCH: &str = "DDUMMY dummy_a dummy_b idealswitchmodel\n\
     DOFFVAL kind=const value=0\n\
     DOFFGATE kind=sig2voltage in=DOFFVAL\n\
     DDUMMY kind=mosfet r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=1e6 g_on=0 gate=block ctrl=DOFFGATE\n\
     Rdummy_a dummy_a 0 1e9\nRdummy_b dummy_b 0 1e9\n";

fn column(stdout: &str, name: &str) -> (Vec<f64>, Vec<f64>) {
    let lines: Vec<&str> = stdout.lines().collect();
    let header: Vec<&str> = lines[0].split(',').collect();
    let t_idx = header.iter().position(|h| *h == "t").unwrap();
    let idx = header.iter().position(|h| *h == name).unwrap();
    let mut ts = Vec::new();
    let mut vs = Vec::new();
    for line in &lines[1..] {
        let cols: Vec<&str> = line.split(',').collect();
        ts.push(cols[t_idx].parse().unwrap());
        vs.push(cols[idx].parse().unwrap());
    }
    (ts, vs)
}

#[test]
fn sinwave_and_sin_coexist_in_one_file_without_collision() {
    // SRC is the new zero-input SFFM-family sinusoid *source* (kind=sinwave); MATHSIN is the
    // pre-existing single-input waveform-arithmetic sin(x) *function* (kind=sin) applied to a
    // plain constant -- both must resolve to their own distinct BlockKind, not shadow one
    // another, even though their CLI keywords differ only by the "wave" suffix.
    let netlist = write_netlist(&format!(
        "{DUMMY_IDEAL_SWITCH}V1 a 0 SRC_V\nR1 a 0 1000\n\
         SRC kind=sinwave v0=0 va=5 freq=1000\n\
         SRC_V kind=sig2voltage in=SRC\n\
         HALFPI kind=const value=1.5707963267948966\n\
         MATHSIN kind=sin in=HALFPI\n"
    ));

    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "5e-4",
        "--dt",
        "1e-6",
    ]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();

    // MATHSIN = sin(pi/2) = 1.0 exactly, every step -- if "sin"/"sinwave" collided, this would
    // instead be missing a required field (va/freq) and the run above would have failed.
    let (_, mathsin) = column(&stdout, "MATHSIN");
    for v in mathsin {
        assert!(
            (v - 1.0).abs() < 1e-9,
            "MATHSIN={v}, expected exactly 1.0 (sin(pi/2))"
        );
    }

    // SRC's own sinusoid still matches the closed form, exactly like the dae-runtime-level
    // signal_domain_sources.rs test already checks (this test's purpose is the CLI-level name
    // resolution, not a fresh derivation).
    let (ts, va) = column(&stdout, "V(a)");
    for (t, v) in ts.iter().zip(va.iter()) {
        let expected = 5.0 * (2.0 * std::f64::consts::PI * 1000.0 * t).sin();
        assert!(
            (v - expected).abs() < 1e-6,
            "t={t}: V(a)={v}, expected {expected}"
        );
    }
}

#[test]
fn pwc_and_pwl_repeat_parse_and_wrap_through_the_real_cli() {
    let netlist = write_netlist(&format!(
        "{DUMMY_IDEAL_SWITCH}V1 a 0 STEP_V\nR1 a 0 1000\nV2 b 0 RAMP_V\nR2 b 0 1000\n\
         STEP kind=pwc points=[[0,1],[0.5e-3,5],[1e-3,1]] repeat=true\n\
         STEP_V kind=sig2voltage in=STEP\n\
         RAMP kind=pwl points=[[0,0],[1e-3,10],[2e-3,0]] repeat=true\n\
         RAMP_V kind=sig2voltage in=RAMP\n"
    ));

    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "3.5e-3",
        "--dt",
        "1e-5",
    ]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();

    let (ts, step) = column(&stdout, "V(a)");
    for (t, v) in ts.iter().zip(step.iter()) {
        let phase = t % 1e-3;
        let expected = if phase < 0.5e-3 { 1.0 } else { 5.0 };
        assert!(
            (v - expected).abs() < 1e-9,
            "t={t}: V(a)={v}, expected {expected} (pwc repeat)"
        );
    }

    let (ts2, ramp) = column(&stdout, "V(b)");
    for (t, v) in ts2.iter().zip(ramp.iter()) {
        let phase = t % 2e-3;
        let expected = if phase <= 1e-3 {
            10.0 * (phase / 1e-3)
        } else {
            10.0 * (2.0 - phase / 1e-3)
        };
        assert!(
            (v - expected).abs() < 1e-6,
            "t={t}: V(b)={v}, expected {expected} (pwl repeat)"
        );
    }
}

#[test]
fn pulsewave_and_expwave_and_sffmwave_parse_through_the_real_cli() {
    let netlist = write_netlist(&format!(
        "{DUMMY_IDEAL_SWITCH}V1 a 0 P_V\nR1 a 0 1000\nV2 b 0 E_V\nR2 b 0 1000\nV3 c 0 F_V\nR3 c 0 1000\n\
         P kind=pulsewave v1=0 v2=10 tr=1e-4 tf=1e-4 pw=2e-4 per=1e-3\n\
         P_V kind=sig2voltage in=P\n\
         E kind=expwave v1=0 v2=1 tau1=1e-3\n\
         E_V kind=sig2voltage in=E\n\
         F kind=sffmwave va=5 fc=1000 mdi=10 fs=100\n\
         F_V kind=sig2voltage in=F\n"
    ));

    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "5e-4",
        "--dt",
        "1e-6",
    ]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();

    let (_, p) = column(&stdout, "V(a)");
    assert!(
        p.iter().any(|&v| v > 9.0),
        "expected pulsewave to reach its v2=10 plateau"
    );

    let (ts, e) = column(&stdout, "V(b)");
    for (t, v) in ts.iter().zip(e.iter()) {
        let expected = 1.0 - (-t / 1e-3).exp();
        assert!(
            (v - expected).abs() < 1e-6,
            "t={t}: V(b)={v}, expected {expected} (expwave)"
        );
    }

    let (ts2, f) = column(&stdout, "V(c)");
    let two_pi = 2.0 * std::f64::consts::PI;
    for (t, v) in ts2.iter().zip(f.iter()) {
        let expected = 5.0 * (two_pi * 1000.0 * t + 10.0 * (two_pi * 100.0 * t).sin()).sin();
        assert!(
            (v - expected).abs() < 1e-9,
            "t={t}: V(c)={v}, expected {expected} (sffmwave)"
        );
    }
}
