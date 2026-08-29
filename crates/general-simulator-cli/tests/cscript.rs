//! Runs the actual built `general-simulator` binary against netlists using `kind=cscript`, compiling
//! the `.c` fixtures in `tests/fixtures/` into real shared libraries at test time (via the
//! system `cc`) -- an end-to-end check through the real CLI parser and `dae-runtime`'s block
//! graph, not just `cscript-ffi`'s own lower-level unit tests. Both fixture netlists use a
//! dummy always-off ideal switch (`gate=block` reading a `Const(0)` wrapped in `sig2voltage`) purely so
//! `simulate_transient_with_blocks`'s code path
//! is reached at all -- `general-simulator-cli` only evaluates the block graph when at least one
//! ideal switch is declared (see `main.rs`'s `run()`); the ideal switch being off never affects R1/the
//! observed cscript output.

use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn compile_fixture(name: &str) -> PathBuf {
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
        "devices-{}.txt",
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
fn cscript_gain_reproduces_a_hand_known_result_every_step() {
    let lib = compile_fixture("cscript_gain");
    let devices = write_devices_file(&format!(
        "OFFVAL kind=const value=0\n\
         OFFGATE kind=sig2voltage in=OFFVAL\n\
         D1 kind=mosfet r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.7 g_on=1 gate=block ctrl=OFFGATE\n\
         SRC kind=const value=5\n\
         CG kind=cscript lib={} in=SRC\n",
        lib.display()
    ));

    let output = run(&[
        fixture("cscript_gain.cir").to_str().unwrap(),
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
    assert!(lines[0].ends_with(",CG"), "header: {}", lines[0]);
    for line in &lines[1..] {
        let cg: f64 = line.split(',').next_back().unwrap().parse().unwrap();
        assert!((cg - 10.0).abs() < 1e-12, "expected CG=10 (5*2), got {cg}");
    }
}

#[test]
fn cscript_sample_time_holds_between_samples_zero_order() {
    let lib = compile_fixture("cscript_counter");
    // dt=0.0001, ts=0.0005 -> the counter should only increment once every 5 rows.
    let devices = write_devices_file(&format!(
        "OFFVAL kind=const value=0\n\
         OFFGATE kind=sig2voltage in=OFFVAL\n\
         D1 kind=mosfet r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.7 g_on=1 gate=block ctrl=OFFGATE\n\
         SRC kind=const value=0\n\
         CNT kind=cscript lib={} in=SRC ts=0.0005\n",
        lib.display()
    ));

    let output = run(&[
        fixture("cscript_counter.cir").to_str().unwrap(),
        "--devices",
        devices.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "0.002",
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
    let counts: Vec<f64> = lines[1..]
        .iter()
        .map(|line| line.split(',').next_back().unwrap().parse().unwrap())
        .collect();

    // Every run of 5 consecutive rows must share the same (held) count, and the count must
    // strictly increase from one run to the next -- exactly the zero-order-hold behavior
    // ts= is documented to give.
    let mut distinct_values = Vec::new();
    for &c in &counts {
        if distinct_values.last() != Some(&c) {
            distinct_values.push(c);
        }
    }
    assert!(
        distinct_values.len() >= 2,
        "counter never advanced: {counts:?}"
    );
    for w in distinct_values.windows(2) {
        assert!(
            w[1] > w[0],
            "counter must strictly increase across samples: {distinct_values:?}"
        );
    }
    // Each held run should be close to 5 rows (dt/ts = 5), not 1 (which would mean ts= was
    // ignored and it sampled every step).
    let mut run_len = 1;
    let mut run_lens = Vec::new();
    for w in counts.windows(2) {
        if w[0] == w[1] {
            run_len += 1;
        } else {
            run_lens.push(run_len);
            run_len = 1;
        }
    }
    for len in &run_lens[1..run_lens.len().saturating_sub(1)] {
        assert_eq!(
            *len, 5,
            "expected 5 held rows per sample, got {len}: {run_lens:?}"
        );
    }
}

#[test]
fn cscript_xc_matches_the_closed_form_step_response() {
    // dx/dt = -K*x + u (K=2.0, see decay_xc.c), driven by a constant u=10 from a zero initial
    // condition (xc always starts at rest -- the same convention every other dynamic block
    // uses): a first-order step response, x(t) = (u/K)*(1 - exp(-K*t)) -- a real, independent
    // closed-form check on the whole xc_count=1 path (netlist parsing -> BlockKind::CScript's
    // xc_count field -> CScriptRegistry::instantiate_xc -> rk4_step_xc/call_xc), not just
    // cscript-ffi's own lower-level unit test against the same fixture.
    let lib = compile_fixture("cscript_decay_xc");
    let devices = write_devices_file(&format!(
        "OFFVAL kind=const value=0\n\
         OFFGATE kind=sig2voltage in=OFFVAL\n\
         D1 kind=mosfet r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.7 g_on=1 gate=block ctrl=OFFGATE\n\
         SRC kind=const value=10\n\
         CG kind=cscript lib={} in=SRC xc_count=1\n",
        lib.display()
    ));

    let output = run(&[
        fixture("cscript_decay_xc.cir").to_str().unwrap(),
        "--devices",
        devices.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "0.01",
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
    assert!(lines[0].ends_with(",CG"), "header: {}", lines[0]);

    let k = 2.0_f64;
    let u = 10.0_f64;
    let dt = 0.0001_f64;
    let mut max_err = 0.0_f64;
    for (i, line) in lines[1..].iter().enumerate() {
        let cg: f64 = line.split(',').next_back().unwrap().parse().unwrap();
        let t = (i as f64 + 1.0) * dt;
        let expected = (u / k) * (1.0 - (-k * t).exp());
        max_err = max_err.max((cg - expected).abs());
    }
    assert!(
        max_err < 1e-6,
        "CG deviates from the closed-form step response by {max_err}"
    );
}

#[test]
fn cscript_without_clone_is_rejected_under_adaptive_step_not_silently_wrong() {
    let lib = compile_fixture("cscript_gain"); // exports no cscript_clone
    let devices = write_devices_file(&format!(
        "OFFVAL kind=const value=0\n\
         OFFGATE kind=sig2voltage in=OFFVAL\n\
         D1 kind=mosfet r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.7 g_on=1 gate=block ctrl=OFFGATE\n\
         SRC kind=const value=5\n\
         CG kind=cscript lib={} in=SRC\n",
        lib.display()
    ));

    // No --dt: adaptive step-size control, the CLI's default.
    let output = run(&[
        fixture("cscript_gain.cir").to_str().unwrap(),
        "--devices",
        devices.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "0.001",
    ]);
    assert!(
        !output.status.success(),
        "expected failure without cscript_clone under adaptive step"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("CScriptRequiresCloneForAdaptiveStep"),
        "stderr should name the specific error, got: {stderr}"
    );
}
