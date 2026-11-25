//! `--format samples`, cross-checked against the CSV of the same run.
//!
//! The CSV path is this project's oldest and most-tested output, so it is
//! the reference here rather than a hand derivation: the same deck written
//! both ways must agree about every signal's extremes. That is the property
//! a lossy writer would break, and it is specifically what a pyramid has to
//! get right -- the stored min/max of every block, folded up, must still be
//! the run's own min and max.
//!
//! The fixture is the PWM-driven buck, chosen because it has ideal switches
//! and blocks and therefore takes the streamed path, which is the one that
//! matters: it is where a run too large to hold is written.

use std::path::PathBuf;
use std::process::Command;

use gsim_samples::{Detail, Reader};

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

/// Column names and each column's extremes, from a CSV this binary printed.
fn csv_extremes(csv: &str) -> (Vec<String>, Vec<(f64, f64)>) {
    let mut lines = csv.lines();
    let headers: Vec<String> = lines
        .next()
        .expect("a header row")
        .split(',')
        .map(str::to_string)
        .collect();
    let mut extremes = vec![(f64::INFINITY, f64::NEG_INFINITY); headers.len()];
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        for (column, field) in line.split(',').enumerate() {
            let value: f64 = field.parse().expect("a number");
            extremes[column].0 = extremes[column].0.min(value);
            extremes[column].1 = extremes[column].1.max(value);
        }
    }
    (headers, extremes)
}

fn out_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name)
}

#[test]
fn every_signal_keeps_the_extremes_the_csv_reports() {
    let netlist = fixture("checkpoint_buck.cir");
    let netlist = netlist.to_str().unwrap();
    let args = ["--mode", "transient", "--tfinal", "3e-4", "--dt", "1e-8"];

    let csv = run(&[&[netlist], &args[..], &["--format", "csv"]].concat());
    let (headers, extremes) = csv_extremes(&csv);

    let samples = out_path("buck_extremes.gsim_samples");
    run(&[
        &[netlist],
        &args[..],
        &["--format", "samples", "--out", samples.to_str().unwrap()],
    ]
    .concat());

    let bytes = std::fs::read(&samples).expect("the samples file");
    let reader = Reader::open(bytes).expect("it opens");

    assert_eq!(
        reader.signals().len(),
        headers.len() - 1,
        "the CSV's columns past `t` are the signals"
    );
    assert_eq!(reader.sweep().name, headers[0]);

    // The stripe summary is the whole pyramid folded to one pair per stripe,
    // so asking for it is asking the coarsest question the file can answer.
    // If any level dropped or shifted a sample, the answer stops matching.
    for (column, name) in headers.iter().enumerate().skip(1) {
        assert_eq!(&reader.signals()[column - 1].name, name);
        let window = reader
            .read_detail(std::slice::from_ref(name), None, Detail::Summary)
            .expect("a summary");
        let low = window.values[0]
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min);
        let high = window.values[0]
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max);
        let (want_low, want_high) = extremes[column];
        let scale = want_high.abs().max(want_low.abs()).max(1.0);
        // f32 storage against the CSV's f64, so the tolerance is f32's.
        assert!(
            (low - want_low).abs() <= scale * 1e-6,
            "{name}: pyramid minimum {low} against the CSV's {want_low}"
        );
        assert!(
            (high - want_high).abs() <= scale * 1e-6,
            "{name}: pyramid maximum {high} against the CSV's {want_high}"
        );
    }
}

#[test]
fn the_rows_written_are_the_rows_the_csv_printed() {
    let netlist = fixture("rc_with_block.cir");
    let netlist = netlist.to_str().unwrap();
    let args = ["--mode", "transient", "--tfinal", "1e-4", "--dt", "1e-7"];

    let csv = run(&[&[netlist], &args[..], &["--format", "csv"]].concat());
    let csv_rows = csv.lines().skip(1).filter(|l| !l.trim().is_empty()).count();

    let samples = out_path("rc_rows.gsim_samples");
    run(&[
        &[netlist],
        &args[..],
        &["--format", "samples", "--out", samples.to_str().unwrap()],
    ]
    .concat());
    let bytes = std::fs::read(&samples).expect("the samples file");
    let reader = Reader::open(bytes).expect("it opens");
    assert_eq!(reader.rows(), csv_rows as u64);
}

#[test]
fn without_out_it_lands_beside_the_netlist() {
    // Same convenience the rawfile has: a binary format needs a
    // destination, and the netlist's own stem is the obvious one, so
    // `--format samples` is a one-flag change.
    let source = fixture("rc_with_block.cir");
    let staged = out_path("default_path_probe.cir");
    std::fs::copy(&source, &staged).expect("staging the netlist");
    let expected = staged.with_extension("gsim_samples");
    let _ = std::fs::remove_file(&expected);

    run(&[
        staged.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "1e-5",
        "--dt",
        "1e-7",
        "--format",
        "samples",
    ]);

    assert!(
        expected.exists(),
        "expected {} to have been written",
        expected.display()
    );
    let bytes = std::fs::read(&expected).expect("the samples file");
    Reader::open(bytes).expect("it opens");
}

#[test]
fn an_unknown_format_lists_the_ones_there_are() {
    let netlist = fixture("rc_with_block.cir");
    let output = Command::new(env!("CARGO_BIN_EXE_general-simulator"))
        .args([netlist.to_str().unwrap(), "--format", "hdf5"])
        .output()
        .expect("failed to run general-simulator binary");
    assert!(!output.status.success(), "a bad --format must fail");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("samples"),
        "the error should name every format: {stderr}"
    );
}
