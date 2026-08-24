//! Every list/matrix-valued `kind=` field (`kind=statespace`'s `a`/`b`/`c`, `kind=tf`'s
//! `num`/`den`, `kind=table`/`kind=pwc`/`kind=pwl`'s `points`) is a real Python list literal —
//! `a=[[1,2],[3,4]]` is exactly `numpy.array([[1,2],[3,4]])`'s own shape, `b=[1,2]` a plain
//! list, `points=[[0,1],[1,2]]` a list of `[x,y]` pairs. This file checks that syntax through
//! the real CLI parser end-to-end (numeric correctness, not just "it parses"), and that the
//! old bare-delimiter syntax (`a=1,2;3,4`, `points=0:1,1:2`) this replaced is now rejected with
//! a clear error rather than silently misparsed.

use std::path::PathBuf;
use std::process::Command;

fn write_netlist(name: &str, contents: &str) -> PathBuf {
    let out_dir = std::env::temp_dir().join("general-simulator-cli-test-fixtures");
    std::fs::create_dir_all(&out_dir).expect("create fixture output dir");
    let path = out_dir.join(format!(
        "python-list-syntax-{name}-{}.cir",
        std::process::id()
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

const DUMMY_MOSFET: &str = "DDUMMY dummy_a dummy_b mosfetmodel\n\
     DOFFVAL kind=const value=0\n\
     DOFFGATE kind=sig2gate in=DOFFVAL\n\
     DDUMMY kind=mosfet r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=1e6 g_on=0 gate=block ctrl=DOFFGATE\n\
     Rdummy_a dummy_a 0 1e9\nRdummy_b dummy_b 0 1e9\n";

fn column(stdout: &str, name: &str) -> Vec<f64> {
    let lines: Vec<&str> = stdout.lines().collect();
    let header: Vec<&str> = lines[0].split(',').collect();
    let idx = header.iter().position(|h| *h == name).unwrap();
    lines[1..]
        .iter()
        .map(|line| line.split(',').nth(idx).unwrap().parse().unwrap())
        .collect()
}

#[test]
fn statespace_a_b_c_parse_as_python_lists_and_match_hand_derived_rc_response() {
    // A single-pole low-pass with tau=1ms: xdot = -1000*x + 1000*u, y = x -- a=[[-1000]],
    // b=[[1000]] (collapsed to a plain vector at this field), c=[[1]]. Step input u=1 -> a
    // textbook first-order response y(t) = 1 - exp(-t/tau).
    let netlist = write_netlist(
        "statespace",
        &format!(
            "{DUMMY_MOSFET}V1 a 0 5\nR1 a 0 1000\n\
             U kind=const value=1\n\
             Y kind=statespace a=[[-1000]] b=[1000] c=[1] d=0 in=U\n"
        ),
    );
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "5e-3",
        "--dt",
        "1e-5",
    ]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    let header: Vec<&str> = lines[0].split(',').collect();
    let t_idx = header.iter().position(|h| *h == "t").unwrap();
    let y = column(&stdout, "Y");
    let ts: Vec<f64> = lines[1..]
        .iter()
        .map(|line| line.split(',').nth(t_idx).unwrap().parse().unwrap())
        .collect();

    let tau = 1e-3;
    let mut max_err = 0.0_f64;
    for (t, y) in ts.iter().zip(y.iter()) {
        let expected = 1.0 - (-t / tau).exp();
        max_err = max_err.max((y - expected).abs());
    }
    assert!(
        max_err < 0.01,
        "Y deviates from the hand-derived first-order step response by {max_err}"
    );
}

#[test]
fn tf_num_den_parse_as_python_lists_and_match_statespace_on_the_same_pole() {
    // Same single-pole plant as above, given instead as a rational tf: 1000/(s+1000).
    let netlist = write_netlist(
        "tf",
        &format!(
            "{DUMMY_MOSFET}V1 a 0 5\nR1 a 0 1000\n\
             U kind=const value=1\n\
             Y kind=tf num=[1000] den=[1,1000] in=U\n"
        ),
    );
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "5e-3",
        "--dt",
        "1e-5",
    ]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    let header: Vec<&str> = lines[0].split(',').collect();
    let t_idx = header.iter().position(|h| *h == "t").unwrap();
    let y = column(&stdout, "Y");
    let ts: Vec<f64> = lines[1..]
        .iter()
        .map(|line| line.split(',').nth(t_idx).unwrap().parse().unwrap())
        .collect();

    let tau = 1e-3;
    let mut max_err = 0.0_f64;
    for (t, y) in ts.iter().zip(y.iter()) {
        let expected = 1.0 - (-t / tau).exp();
        max_err = max_err.max((y - expected).abs());
    }
    assert!(
        max_err < 0.01,
        "Y deviates from the hand-derived first-order step response by {max_err}"
    );
}

#[test]
fn table_points_parse_as_a_python_list_of_xy_pairs_and_interpolate_correctly() {
    let netlist = write_netlist(
        "table",
        &format!(
            "{DUMMY_MOSFET}V1 a 0 5\nR1 a 0 1000\n\
             X kind=const value=1.5\n\
             Y kind=table points=[[0,0],[1,10],[2,20]] in=X\n"
        ),
    );
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "1e-4",
        "--dt",
        "1e-5",
    ]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let y = column(&stdout, "Y");
    // Linear interpolation between (1,10) and (2,20) at x=1.5 -> 15.0, exactly, every step.
    for v in y {
        assert!((v - 15.0).abs() < 1e-9, "Y={v}, expected exactly 15.0");
    }
}

#[test]
fn old_a reference tool_style_matrix_syntax_is_rejected_with_a_clear_error() {
    let netlist = write_netlist(
        "old-matrix",
        &format!(
            "{DUMMY_MOSFET}V1 a 0 5\nR1 a 0 1000\n\
             U kind=const value=1\n\
             Y kind=statespace a=-1000 b=1000 c=1 d=0 in=U\n"
        ),
    );
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "1e-4",
        "--dt",
        "1e-5",
    ]);
    assert!(
        !output.status.success(),
        "expected the old syntax to be rejected"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("Python-style list"),
        "stderr should explain the expected Python-list syntax, got: {stderr}"
    );
}

#[test]
fn old_colon_pair_points_syntax_is_rejected_with_a_clear_error() {
    let netlist = write_netlist(
        "old-points",
        &format!(
            "{DUMMY_MOSFET}V1 a 0 5\nR1 a 0 1000\n\
             X kind=const value=1.5\n\
             Y kind=table points=0:0,1:10,2:20 in=X\n"
        ),
    );
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "1e-4",
        "--dt",
        "1e-5",
    ]);
    assert!(
        !output.status.success(),
        "expected the old syntax to be rejected"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("Python-style list"),
        "stderr should explain the expected Python-list syntax, got: {stderr}"
    );
}

#[test]
fn nan_in_a_list_field_is_a_clean_error_not_a_process_panic() {
    // f64::from_str accepts "nan" -- without an explicit finiteness check this used to reach a
    // partial_cmp().unwrap() in the points sort and panic the whole process instead of failing
    // this one netlist line. Exercised through kind=pwc's own points= (the sort is specific to
    // that field), not just parse_vector in isolation.
    let netlist = write_netlist(
        "nan-points",
        &format!(
            "{DUMMY_MOSFET}V1 a 0 5\nR1 a 0 1000\n\
             REF kind=pwc points=[[nan,0],[1,2]]\n"
        ),
    );
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "1e-4",
        "--dt",
        "1e-5",
    ]);
    assert!(
        !output.status.success(),
        "expected a NaN entry to be rejected, not accepted"
    );
    assert!(
        output.status.code().is_some(),
        "expected a clean process exit (error code), not a panic/abort"
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("finite"),
        "stderr should explain the value must be finite, got: {stderr}"
    );
}
