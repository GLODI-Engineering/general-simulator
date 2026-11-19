//! `BlockKind::FileWave` — a measured waveform, evaluated.
//!
//! `general-mna` loads the capture and this crate evaluates it, so these tests are about the
//! evaluation: the interpolation, what happens past the capture's last sample, and the second
//! channel being readable by its own name.
//!
//! The identity that matters most is the last test here: a file and an inline `points=` list of
//! the same numbers must drive a circuit to **exactly** the same voltages. If they ever differ,
//! there are two implementations of a sampled waveform rather than one, and a user comparing a
//! simulation to a measurement would be comparing it to an artefact of which spelling they
//! happened to use.

use std::collections::BTreeMap;
use std::path::PathBuf;

use dae_runtime::{
    simulate_transient_with_blocks, BlockInstance, BlockKind, DaeError, PhysicalDomain, Signal,
    TimeStep, WaveAfter, WaveInterp,
};
use general_spice_core::Dialect;
use pwl_devices::{IdealDiode, IdealSwitch};

/// `V1 a 0 CMD_V` through a bare resistor, so `V(a)` is the source block's own output exactly,
/// every step — the identity-check pattern the other signal-source tests use.
fn run(kind: BlockKind, extra: Vec<BlockInstance>, t_final: f64, dt: f64) -> Vec<(f64, f64)> {
    trace(kind, extra, t_final, dt).expect("the run must succeed")
}

fn trace(
    kind: BlockKind,
    extra: Vec<BlockInstance>,
    t_final: f64,
    dt: f64,
) -> Result<Vec<(f64, f64)>, DaeError> {
    let netlist = "V1 a 0 CMD_V\nR1 a 0 1000";
    let ideal_switches: BTreeMap<String, IdealSwitch> = BTreeMap::new();
    let diodes: BTreeMap<String, IdealDiode> = BTreeMap::new();
    let gates = BTreeMap::new();

    let mut blocks = vec![
        BlockInstance {
            name: "CMD".to_string(),
            kind,
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "CMD_V".to_string(),
            kind: BlockKind::Sig2Phys {
                domain: PhysicalDomain::Voltage,
            },
            inputs: vec![Signal::Block("CMD".to_string())],
            ic: None,
        },
    ];
    blocks.extend(extra);

    simulate_transient_with_blocks(
        netlist,
        Dialect::Ngspice,
        &diodes,
        &ideal_switches,
        &blocks,
        &gates,
        0.1,
        None,
        t_final,
        TimeStep::Fixed(dt),
    )
    .map(|steps| {
        steps
            .iter()
            .map(|(t, point, _)| (*t, point.value("V(a)").unwrap()))
            .collect()
    })
}

fn file_wave(
    columns: Vec<Vec<(f64, f64)>>,
    names: &[&str],
    interp: WaveInterp,
    after: WaveAfter,
) -> BlockKind {
    BlockKind::FileWave {
        columns,
        output_names: names.iter().map(|name| name.to_string()).collect(),
        interp,
        after,
        source: PathBuf::from("capture.csv"),
    }
}

fn at(trace: &[(f64, f64)], t: f64) -> f64 {
    trace
        .iter()
        .min_by(|a, b| (a.0 - t).abs().partial_cmp(&(b.0 - t).abs()).unwrap())
        .map(|(_, v)| *v)
        .unwrap_or_else(|| panic!("no sample near {t}"))
}

/// `interp=linear` is PWL: interpolated between the capture's samples.
#[test]
fn a_capture_interpolates_between_its_samples() {
    let trace = run(
        file_wave(
            vec![vec![(0.0, 0.0), (1e-3, 10.0)]],
            &["CMD"],
            WaveInterp::Linear,
            WaveAfter::Hold,
        ),
        vec![],
        1e-3,
        1e-4,
    );
    // Halfway through the ramp, halfway up it -- not the previous sample's value.
    assert!(
        (at(&trace, 5e-4) - 5.0).abs() < 1e-9,
        "{:?}",
        at(&trace, 5e-4)
    );
}

/// `interp=hold` is PWC: the last sample's value, which is what a register value printed by a
/// microcontroller is. The same capture read the other way is a different waveform, deliberately.
#[test]
fn a_capture_can_hold_instead_of_interpolating() {
    let trace = run(
        file_wave(
            vec![vec![(0.0, 0.0), (1e-3, 10.0)]],
            &["CMD"],
            WaveInterp::Hold,
            WaveAfter::Hold,
        ),
        vec![],
        1e-3,
        1e-4,
    );
    assert!(
        (at(&trace, 5e-4) - 0.0).abs() < 1e-9,
        "{:?}",
        at(&trace, 5e-4)
    );
}

/// Past the last sample, each `after` does its own thing. Asserted together because the
/// difference between them is the whole point of the option.
#[test]
fn past_the_last_sample_hold_repeat_and_zero_each_do_their_own_thing() {
    let capture = vec![vec![(0.0, 0.0), (1e-3, 10.0)]];

    let held = run(
        file_wave(
            capture.clone(),
            &["CMD"],
            WaveInterp::Linear,
            WaveAfter::Hold,
        ),
        vec![],
        3e-3,
        1e-4,
    );
    assert!((at(&held, 2e-3) - 10.0).abs() < 1e-9, "{}", at(&held, 2e-3));

    let zeroed = run(
        file_wave(
            capture.clone(),
            &["CMD"],
            WaveInterp::Linear,
            WaveAfter::Zero,
        ),
        vec![],
        3e-3,
        1e-4,
    );
    assert!(
        (at(&zeroed, 2e-3) - 0.0).abs() < 1e-9,
        "{}",
        at(&zeroed, 2e-3)
    );

    let repeated = run(
        file_wave(capture, &["CMD"], WaveInterp::Linear, WaveAfter::Repeat),
        vec![],
        3e-3,
        1e-4,
    );
    // One period on, the ramp is back where it was halfway up.
    assert!(
        (at(&repeated, 1.5e-3) - 5.0).abs() < 1e-9,
        "{}",
        at(&repeated, 1.5e-3)
    );
}

/// `after=error` refuses the run **before it starts**, naming the capture's end and the run's
/// own. Past its data a file source holds a constant, so a run that outlasts its capture is
/// driven by a flat line for the rest of its length and the waveform looks entirely plausible.
#[test]
fn a_run_that_outlasts_its_capture_is_refused_before_it_starts() {
    let error = trace(
        file_wave(
            vec![vec![(0.0, 0.0), (1e-3, 10.0)]],
            &["CMD"],
            WaveInterp::Linear,
            WaveAfter::Error,
        ),
        vec![],
        5e-3,
        1e-4,
    )
    .unwrap_err();

    match error {
        DaeError::WaveCaptureTooShort {
            block,
            source,
            covered,
            t_final,
        } => {
            assert_eq!(block, "CMD");
            assert_eq!(source, "capture.csv");
            assert_eq!(covered, 1e-3);
            assert_eq!(t_final, 5e-3);
        }
        other => panic!("expected WaveCaptureTooShort, got {other:?}"),
    }

    // And a run the capture does cover is allowed, so the check is a bound and not a ban.
    let ok = trace(
        file_wave(
            vec![vec![(0.0, 0.0), (1e-3, 10.0)]],
            &["CMD"],
            WaveInterp::Linear,
            WaveAfter::Error,
        ),
        vec![],
        1e-3,
        1e-4,
    );
    assert!(ok.is_ok(), "{:?}", ok.unwrap_err());
}

/// A two-channel capture: the first channel is the block's own value and the second is a
/// top-level signal name of its own. Driven here through a second `sig2phys` so the test reads
/// the second channel the way a netlist would, not by inspecting the outputs map.
#[test]
fn the_second_channel_is_readable_by_its_own_name() {
    let trace = run(
        file_wave(
            vec![vec![(0.0, 1.0), (1e-3, 1.0)], vec![(0.0, 7.0), (1e-3, 7.0)]],
            &["CMD", "CH2"],
            WaveInterp::Linear,
            WaveAfter::Hold,
        ),
        vec![
            // Reads the second channel by name and is never consumed -- its presence alone
            // proves the name resolves, since an unresolved `Signal::Block` is a hard error.
            BlockInstance {
                name: "SECOND".to_string(),
                kind: BlockKind::Gain(dae_runtime::GainValue::Scalar(1.0)),
                inputs: vec![Signal::Block("CH2".to_string())],
                ic: None,
            },
        ],
        1e-3,
        1e-4,
    );
    // The first channel still drives the circuit.
    assert!(
        (at(&trace, 5e-4) - 1.0).abs() < 1e-9,
        "{}",
        at(&trace, 5e-4)
    );
}

/// **The identity that matters.** A capture and an inline `points=` list of the same numbers
/// must drive the circuit to exactly the same voltages -- not nearly. Two spellings of one
/// waveform that disagreed would mean a user comparing a simulation to a measurement was partly
/// comparing it to whichever spelling they used.
#[test]
fn a_file_and_an_inline_points_list_of_the_same_numbers_agree_exactly() {
    let points = vec![
        (0.0, 0.0),
        (2e-4, 3.0),
        (4e-4, -1.5),
        (7e-4, 9.0),
        (1e-3, 0.0),
    ];
    let from_file = run(
        file_wave(
            vec![points.clone()],
            &["CMD"],
            WaveInterp::Linear,
            WaveAfter::Hold,
        ),
        vec![],
        1e-3,
        1e-5,
    );
    let inline = run(
        BlockKind::Pwl {
            points,
            repeat: false,
        },
        vec![],
        1e-3,
        1e-5,
    );
    assert_eq!(from_file.len(), inline.len());
    for ((t_file, v_file), (t_inline, v_inline)) in from_file.iter().zip(inline.iter()) {
        assert_eq!(t_file, t_inline);
        assert_eq!(
            v_file, v_inline,
            "at t = {t_file}: a file gave {v_file}, an inline list gave {v_inline}"
        );
    }
}

/// 100 000 samples over 20 000 steps. The scan this replaced would be ~2x10^9 comparisons for
/// this one source; the lookup is `O(log n)`, so it finishes in well under a second of block
/// evaluation. A guard against the scan coming back, not a benchmark.
#[test]
fn a_deep_memory_capture_does_not_scan_itself_every_step() {
    let points: Vec<(f64, f64)> = (0..100_000)
        .map(|index| {
            let t = index as f64 * 1e-8;
            (t, (index % 100) as f64 / 100.0)
        })
        .collect();
    let started = std::time::Instant::now();
    let trace = run(
        file_wave(vec![points], &["CMD"], WaveInterp::Linear, WaveAfter::Hold),
        vec![],
        2e-4,
        1e-8,
    );
    // The row count is the stepper's business, not this test's -- what matters is that the
    // whole run happened and the last row is where it should be.
    assert!(trace.len() > 19_000, "{} rows", trace.len());
    assert!((trace.last().unwrap().0 - 2e-4).abs() < 1e-9);
    assert!(
        started.elapsed().as_secs() < 30,
        "a 100k-sample capture over 20k steps took {:?}",
        started.elapsed()
    );
}
