//! Runs the actual built `general-simulator` binary as a subprocess (same pattern as `cli.rs`)
//! against the same `rc_diode` fixture already hand-verified there, once with `--format csv`
//! (the default) and once with `--format raw`, and asserts the two really are "the same data,
//! just serialized differently" — same variable names (`t` renamed to `time`, the rest
//! unchanged), same row count, same values. This crate's own `src/raw_format.rs` has the
//! self-consistency round-trip test; this file is the CLI-level proof that `--format`/`--out`
//! wiring in `main.rs` actually reaches it with the same `Waveform` the CSV path uses. The
//! deeper cross-validation against real third-party SPICE-rawfile readers (PySpice, `spicelib`)
//! lives in `doc-verify/raw-output/` — see its own README.

use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_general-simulator"))
        .args(args)
        .output()
        .expect("failed to run general-simulator binary")
}

fn run_ok(args: &[&str]) -> String {
    let output = run(args);
    assert!(
        output.status.success(),
        "general-simulator exited with {:?}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout was not valid UTF-8")
}

/// Minimal binary-rawfile parser, deliberately not reusing `raw_format::write_raw`'s own
/// internals -- a reader built from the writer's own code would just prove the writer agrees
/// with itself, not that the file is actually structured the way it claims to be.
struct ParsedRaw {
    header: std::collections::HashMap<String, String>,
    variables: Vec<(usize, String, String)>,
    rows: Vec<Vec<f64>>,
}

fn parse_raw(bytes: &[u8]) -> ParsedRaw {
    let marker = b"Binary:\n";
    let binary_pos = bytes
        .windows(marker.len())
        .position(|w| w == marker)
        .expect("Binary: marker present");
    let data_start = binary_pos + marker.len();
    let header_text = std::str::from_utf8(&bytes[..binary_pos]).expect("header is valid UTF-8");

    let mut header = std::collections::HashMap::new();
    let mut variables = Vec::new();
    let mut in_variables = false;
    for line in header_text.lines() {
        if line == "Variables:" {
            in_variables = true;
            continue;
        }
        if in_variables {
            let parts: Vec<&str> = line.trim().split('\t').collect();
            assert_eq!(
                parts.len(),
                3,
                "variable line should be idx\\tname\\ttype: {line:?}"
            );
            let idx: usize = parts[0].parse().expect("variable index parses as usize");
            variables.push((idx, parts[1].to_string(), parts[2].to_string()));
        } else if let Some((k, v)) = line.split_once(':') {
            header.insert(k.trim().to_string(), v.trim().to_string());
        }
    }

    let n_vars: usize = header["No. Variables"].parse().unwrap();
    let n_points: usize = header["No. Points"].parse().unwrap();
    assert_eq!(variables.len(), n_vars);

    let data = &bytes[data_start..];
    assert_eq!(data.len(), n_vars * n_points * 8);
    let mut rows = Vec::with_capacity(n_points);
    let mut offset = 0;
    for _ in 0..n_points {
        let mut row = Vec::with_capacity(n_vars);
        for _ in 0..n_vars {
            let mut b = [0u8; 8];
            b.copy_from_slice(&data[offset..offset + 8]);
            row.push(f64::from_le_bytes(b));
            offset += 8;
        }
        rows.push(row);
    }

    ParsedRaw {
        header,
        variables,
        rows,
    }
}

#[test]
fn raw_output_matches_csv_output_for_the_same_transient_run() {
    let netlist = fixture("rc_diode.cir");
    let devices = fixture("rc_diode_devices.txt");
    let out_dir = std::env::temp_dir().join(format!(
        "general-simulator-raw-cli-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&out_dir).unwrap();
    let raw_path = out_dir.join("rc_diode.raw");

    let csv = run_ok(&[
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

    let raw_stdout = run_ok(&[
        netlist.to_str().unwrap(),
        "--devices",
        devices.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "2.0",
        "--dt",
        "0.001",
        "--format",
        "raw",
        "--out",
        raw_path.to_str().unwrap(),
    ]);
    assert!(
        raw_stdout.is_empty(),
        "--format raw should not print CSV to stdout"
    );

    let csv_lines: Vec<&str> = csv.lines().collect();
    let csv_headers: Vec<&str> = csv_lines[0].split(',').collect();
    assert_eq!(csv_headers, vec!["t", "V(a)", "V(b)", "V(c)", "I(V1)"]);
    let csv_rows: Vec<Vec<f64>> = csv_lines[1..]
        .iter()
        .map(|line| line.split(',').map(|f| f.parse().unwrap()).collect())
        .collect();

    let raw_bytes = std::fs::read(&raw_path).expect("raw file was written");
    let parsed = parse_raw(&raw_bytes);

    assert_eq!(parsed.header["Flags"], "real");
    assert_eq!(parsed.header["Plotname"], "Transient Analysis");
    assert_eq!(
        parsed.header["No. Variables"].parse::<usize>().unwrap(),
        csv_headers.len()
    );
    assert_eq!(
        parsed.header["No. Points"].parse::<usize>().unwrap(),
        csv_rows.len()
    );

    // Same variable set, same order: `t` -> `time` (the SPICE convention this crate mirrors),
    // everything else unchanged from its CSV column name.
    let raw_names: Vec<&str> = parsed
        .variables
        .iter()
        .map(|(_, n, _)| n.as_str())
        .collect();
    assert_eq!(raw_names, vec!["time", "V(a)", "V(b)", "V(c)", "I(V1)"]);
    let raw_types: Vec<&str> = parsed
        .variables
        .iter()
        .map(|(_, _, t)| t.as_str())
        .collect();
    assert_eq!(
        raw_types,
        vec!["time", "voltage", "voltage", "voltage", "current"]
    );
    for (i, (idx, _, _)) in parsed.variables.iter().enumerate() {
        assert_eq!(*idx, i);
    }

    // Same row count and, row by row, the exact same values (this crate writes CSV via `f64`'s
    // `Display`, so parsing it back and comparing against the raw file's own doubles is an exact
    // comparison, not a tolerance-based one -- both trace back to the same in-memory `f64`).
    assert_eq!(parsed.rows.len(), csv_rows.len());
    for (raw_row, csv_row) in parsed.rows.iter().zip(csv_rows.iter()) {
        assert_eq!(raw_row, csv_row);
    }

    std::fs::remove_dir_all(&out_dir).ok();
}

#[test]
fn format_raw_without_out_defaults_to_netlist_stem_dot_raw_next_to_the_input() {
    // Copy the fixture into a scratch dir so this test doesn't leave a stray `.raw` file next to
    // the committed fixture, and so a parallel test run never collides on the default path.
    let out_dir = std::env::temp_dir().join(format!(
        "general-simulator-raw-default-path-test-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&out_dir).unwrap();
    let netlist_copy = out_dir.join("two_diode.cir");
    let devices_copy = out_dir.join("two_diode_devices.txt");
    std::fs::copy(fixture("two_diode.cir"), &netlist_copy).unwrap();
    std::fs::copy(fixture("two_diode_devices.txt"), &devices_copy).unwrap();

    let expected_default = out_dir.join("two_diode.raw");
    assert!(!expected_default.exists());

    let stdout = run_ok(&[
        netlist_copy.to_str().unwrap(),
        "--devices",
        devices_copy.to_str().unwrap(),
        "--mode",
        "dc",
        "--format",
        "raw",
    ]);
    assert!(stdout.is_empty());
    assert!(
        expected_default.exists(),
        "expected default raw output path {} to exist",
        expected_default.display()
    );

    let parsed = parse_raw(&std::fs::read(&expected_default).unwrap());
    assert_eq!(parsed.header["Plotname"], "DC transfer characteristic");
    assert_eq!(parsed.rows.len(), 1);

    std::fs::remove_dir_all(&out_dir).ok();
}

#[test]
fn unknown_format_flag_is_rejected_with_a_clear_error() {
    let netlist = fixture("two_diode.cir");
    let devices = fixture("two_diode_devices.txt");
    let output = run(&[
        netlist.to_str().unwrap(),
        "--devices",
        devices.to_str().unwrap(),
        "--mode",
        "dc",
        "--format",
        "yaml",
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown --format 'yaml'"),
        "stderr: {stderr}"
    );
}
