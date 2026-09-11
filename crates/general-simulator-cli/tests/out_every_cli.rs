//! `--out-every N` thins the output and nothing else.
//!
//! The claim the flag makes is narrow and worth testing as stated: the rows that *are* written
//! are byte-for-byte the rows the undecimated run wrote at those same indices, so the solution
//! is unaffected and only serialization is skipped. Testing "the file got smaller" would pass
//! for an implementation that coarsened the timestep instead, which is precisely the mistake
//! this flag exists to avoid.

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
        "general-simulator exited with {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout was not valid UTF-8")
}

fn transient(extra: &[&str]) -> String {
    let netlist = fixture("rc_diode.cir");
    let devices = fixture("rc_diode_devices.txt");
    let mut args = vec![
        netlist.to_str().unwrap().to_string(),
        "--devices".into(),
        devices.to_str().unwrap().to_string(),
        "--mode".into(),
        "transient".into(),
        "--tfinal".into(),
        "1.0".into(),
        "--dt".into(),
        "0.01".into(),
    ];
    args.extend(extra.iter().map(|s| s.to_string()));
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run(&refs)
}

#[test]
fn out_every_keeps_exactly_the_rows_the_full_run_produced_at_those_indices() {
    let full: Vec<String> = transient(&[]).lines().map(str::to_string).collect();
    let thinned: Vec<String> = transient(&["--out-every", "10"])
        .lines()
        .map(str::to_string)
        .collect();

    assert_eq!(full[0], thinned[0], "the header must be unaffected");

    let full_rows = &full[1..];
    let thinned_rows = &thinned[1..];
    let expected: Vec<&String> = full_rows.iter().step_by(10).collect();
    assert_eq!(
        thinned_rows.len(),
        expected.len(),
        "decimated row count should be ceil(n/10)"
    );
    for (got, want) in thinned_rows.iter().zip(expected) {
        assert_eq!(got, want, "a kept row must be identical to the full run's");
    }
    assert!(
        full_rows.len() > thinned_rows.len(),
        "the fixture must be long enough for decimation to be observable"
    );
}

#[test]
fn out_every_one_is_byte_for_byte_the_default() {
    assert_eq!(transient(&[]), transient(&["--out-every", "1"]));
}

#[test]
fn out_every_zero_is_rejected() {
    let netlist = fixture("rc_diode.cir");
    let devices = fixture("rc_diode_devices.txt");
    let output = Command::new(env!("CARGO_BIN_EXE_general-simulator"))
        .args([
            netlist.to_str().unwrap(),
            "--devices",
            devices.to_str().unwrap(),
            "--mode",
            "transient",
            "--tfinal",
            "1.0",
            "--dt",
            "0.01",
            "--out-every",
            "0",
        ])
        .output()
        .expect("failed to run general-simulator binary");
    assert!(!output.status.success(), "--out-every 0 must be an error");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--out-every must be at least 1"),
        "unhelpful message: {stderr}"
    );
}

/// The streaming path (any netlist with a block or an ideal switch) emits rows one at a time
/// from inside the solver callback rather than decimating a finished waveform, so it is a
/// separate implementation of the same rule and needs its own test. This is the path every
/// switched-converter deck takes, and the one the flag was added for.
#[test]
fn out_every_applies_on_the_streaming_path_too() {
    let netlist = fixture("rc_with_block.cir");
    let base = |extra: &[&str]| {
        let mut args = vec![
            netlist.to_str().unwrap().to_string(),
            "--mode".into(),
            "transient".into(),
            "--tfinal".into(),
            "1.0".into(),
            "--dt".into(),
            "0.01".into(),
        ];
        args.extend(extra.iter().map(|s| s.to_string()));
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        run(&refs)
    };

    let full: Vec<String> = base(&[]).lines().map(str::to_string).collect();
    let thinned: Vec<String> = base(&["--out-every", "10"])
        .lines()
        .map(str::to_string)
        .collect();

    assert_eq!(full[0], thinned[0], "the header must be unaffected");
    let expected: Vec<&String> = full[1..].iter().step_by(10).collect();
    assert_eq!(thinned[1..].len(), expected.len());
    for (got, want) in thinned[1..].iter().zip(expected) {
        assert_eq!(got, want, "a kept row must be identical to the full run's");
    }
    assert!(full[1..].len() > thinned[1..].len());
}
