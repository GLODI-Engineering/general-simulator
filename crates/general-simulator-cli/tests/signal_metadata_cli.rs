//! What every signal of a run reports about itself, for a deck that has both kinds of signal in
//! it — see `book/user-guide/src/signal-metadata.md` for the convention and
//! `general-simulator-cli`'s `signal_meta` module for how it is derived.
//!
//! The fixture is `signal_metadata.cir`: three node voltages, three branch currents, two
//! `phys2sig` probes (one on a node, one on a branch), two ordinary signal-flow blocks whose
//! unit nothing in the netlist fixes, and one `sig2phys` driving a source. The point of pinning
//! all of it in one table is that the interesting answer is not any single row but the
//! *difference* between the rows: `DUTY` must not come back wearing volts just because the run
//! that produced it is full of them.

use std::path::PathBuf;
use std::process::Command;

use gsim_samples::Reader;

/// `(name, quantity, unit)` of one vector, as the footer reports it.
type VectorRow = (String, String, String);

fn out_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name)
}

/// The fixture deck, copied to its own path under the test target directory.
///
/// Per test rather than read in place: the deck has `kind=measure` lines, so every run writes a
/// `<stem>.log` *beside the netlist*, and three tests reading one fixture would race over one
/// log file. Each test gets its own stem and its own log.
fn netlist_copy(stem: &str) -> PathBuf {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join("signal_metadata.cir");
    let target = out_path(&format!("{stem}.cir"));
    std::fs::copy(&source, &target).expect("copying the fixture deck");
    target
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

/// `(name, quantity, unit)` for every signal, and the sweep axis, that a `.gsim_samples` footer
/// reports after a run of the fixture.
fn footer_table(stem: &str) -> (Vec<VectorRow>, VectorRow) {
    let netlist = netlist_copy(stem);
    let samples = out_path(&format!("{stem}.gsim_samples"));
    run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "3e-3",
        "--dt",
        "1e-3",
        "--format",
        "samples",
        "--out",
        samples.to_str().unwrap(),
    ]);
    let bytes = std::fs::read(&samples).expect("the samples file");
    let reader = Reader::open(bytes).expect("it opens");
    let row = |m: &gsim_samples::VectorMeta| (m.name.clone(), m.quantity.clone(), m.unit.clone());
    (
        reader.signals().iter().map(row).collect(),
        row(reader.sweep()),
    )
}

#[test]
fn every_signal_says_what_kind_of_quantity_it_is() {
    let (signals, sweep) = footer_table("signal_metadata_footer");

    assert_eq!(
        sweep,
        ("t".into(), "time".into(), "s".into()),
        "the sweep axis is seconds and the footer says so"
    );

    let expected: Vec<(&str, &str, &str)> = vec![
        // The circuit's own unknowns. `general-mna` generated these names, one per node then
        // one per branch, so what they are is not in question.
        ("V(a)", "voltage", "V"),
        ("V(b)", "voltage", "V"),
        ("V(d)", "voltage", "V"),
        ("I(V1)", "current", "A"),
        ("I(L1)", "current", "A"),
        ("I(V2)", "current", "A"),
        // A `phys2sig` names the node or the branch it reads, which fixes its unit even though
        // it is a block output.
        ("VPROBE", "voltage", "V"),
        ("IPROBE", "current", "A"),
        // Signal-flow values. `DUTY` is 0.42 of a period and `SCALED` is twice that; nothing in
        // the netlist says what either one measures, so the unit is empty and `signal` is what
        // carries the meaning. Labelling these `V` is the bug this convention exists to stop.
        ("DUTY", "signal", ""),
        ("SCALED", "signal", ""),
        // A `sig2phys` declares the domain it drives.
        ("DRIVE", "voltage", "V"),
    ];

    let actual: Vec<(&str, &str, &str)> = signals
        .iter()
        .map(|(n, q, u)| (n.as_str(), q.as_str(), u.as_str()))
        .collect();
    assert_eq!(actual, expected);
}

/// An empty `unit` is the convention's way of saying "not known", so no signal may carry a unit
/// the netlist did not fix, and every signal must still say *what kind of thing* it is. Asserted
/// separately from the table above because it is the rule, and the table is only one deck's
/// worth of evidence for it.
#[test]
fn no_signal_is_left_without_a_quantity_and_none_invents_a_unit() {
    let (signals, sweep) = footer_table("signal_metadata_rules");
    for (name, quantity, unit) in signals.iter().chain(std::iter::once(&sweep)) {
        assert!(
            ["time", "voltage", "current", "signal"].contains(&quantity.as_str()),
            "{name} reports quantity {quantity:?}, which is not in the convention's vocabulary"
        );
        let expected_unit = match quantity.as_str() {
            "time" => "s",
            "voltage" => "V",
            "current" => "A",
            _ => "",
        };
        assert_eq!(
            unit, expected_unit,
            "{name} is a {quantity} and may only report {expected_unit:?} as its unit"
        );
    }
}

/// The names are the interface, so they do not change: the CSV header of the same run is what it
/// has always been, and a `kind=measure` statement naming a block output column still resolves.
/// This is the compatibility half of the convention, and it is the half a name-encoded unit
/// would have broken.
#[test]
fn names_do_not_change_and_a_measure_reference_still_resolves() {
    let netlist = netlist_copy("signal_metadata_names");
    let csv = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "3e-3",
        "--dt",
        "1e-3",
    ]);
    let header = csv.lines().next().expect("a header row");
    assert_eq!(
        header, "t,V(a),V(b),V(d),I(V1),I(L1),I(V2),VPROBE,IPROBE,DUTY,SCALED,DRIVE",
        "no unit, prefix or suffix is encoded into a column name"
    );

    // `MDUTY` names `DUTY` (a signal-flow column) and `MVB` names `V(b)` (a circuit one); both
    // are plain column names, resolved exactly as before.
    let log = netlist.with_extension("log");
    let text = std::fs::read_to_string(&log).expect("the measurements log");
    assert!(text.contains("MDUTY = 0.42"), "{text}");
    assert!(text.contains("MVB = 0.00125"), "{text}");
    let _ = std::fs::remove_file(&log);
}
