//! Regression test for the previously-demonstrated bug (see
//! `internal-archive/experiments/elspice-pwl-ps-pwm-leg-modulator/`): under
//! `TimeStep::Adaptive`, once `dt_max` approaches or exceeds a `PhaseShiftPwm` modulator's own
//! switching period, real gate edges were **silently missed** (not just jittered) — the adaptive
//! controller had zero awareness of any block's "when do I next change" information, only ever
//! detecting a gate change *after* a trial step had already landed past it. If two edges fell
//! inside one trial step, both transitions collapsed into a single detected change and a pulse
//! vanished outright.
//!
//! `earliest_next_event_dt` (`crates/dae-runtime/src/block_graph.rs`) fixes this by clamping
//! every outer adaptive-loop trial `dt` to the earliest predicted edge, computed in closed form
//! from the modulator's own persisted phase/last-evaluated inputs, *before* the LTE-driven
//! retry loop ever runs. This test proves the fix directly: it drives an ideal switch with a
//! `PhaseShiftPwm` modulator (fixed 600kHz, 40% duty, 2% dead time on each edge) into a quiet RC
//! load (`RC = 100us >> 1.667us period`, so nothing about the circuit's own LTE would otherwise
//! force a small step near an edge), runs the *same* netlist and modulator under `TimeStep::
//! Adaptive` at the three `dt_max` levels that previously demonstrated missed edges in the
//! sibling experiment (`0.48x`, `0.9x`, and `3x` the switching period), and compares each run's
//! own edge count (transitions of the `MOD` block's own output) against a fine fixed-step ground
//! truth over the same `t_final` — asserting **zero missed edges** at every level.

use std::collections::BTreeMap;

use dae_runtime::{
    simulate_transient_with_blocks, AdaptiveConfig, BlockInstance, BlockKind, ConstValue,
    GateBinding, PhysicalDomain, Signal, SignalValue, TimeStep, TransientWithBlocksStep,
};
use general_spice_core::Dialect;
use pwl_devices::{IdealDiode, IdealSwitch};

const FSW: f64 = 600_000.0;
const PERIOD: f64 = 1.0 / FSW;
const DUTY: f64 = 0.4;
const DEAD_FRAC: f64 = 0.02;
const NUM_PERIODS: f64 = 40.0;
const T_FINAL: f64 = NUM_PERIODS * PERIOD;

fn netlist() -> &'static str {
    // V1 -- D1 (ideal switch) -- R1 -- C1 -- ground: RC = 1k * 100n = 100us, ~60x the switching
    // period, so the load is "quiet" -- an LTE-only adaptive controller would happily take huge
    // steps across many periods if not for the event clamp under test.
    "V1 a 0 10\nD1 a b idealswitchmodel\nR1 b c 1k\nC1 c 0 100n"
}

fn blocks() -> Vec<BlockInstance> {
    vec![
        BlockInstance {
            name: "FREQ".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(FSW)),
            inputs: vec![],
        },
        BlockInstance {
            name: "PHASE".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(0.0)),
            inputs: vec![],
        },
        BlockInstance {
            name: "DUTY".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(DUTY)),
            inputs: vec![],
        },
        BlockInstance {
            name: "MOD".to_string(),
            kind: BlockKind::PhaseShiftPwm {
                osc: continuous_blocks::Vco::new(FSW, FSW).unwrap(),
                red: DEAD_FRAC * PERIOD,
                fed: DEAD_FRAC * PERIOD,
                output_names: vec!["MOD".to_string(), "MOD_COMP".to_string()],
            },
            inputs: vec![
                Signal::Block("FREQ".to_string()),
                Signal::Block("PHASE".to_string()),
                Signal::Block("DUTY".to_string()),
            ],
        },
        BlockInstance {
            name: "MOD_MAIN_GATE".to_string(),
            kind: BlockKind::Sig2Phys {
                domain: PhysicalDomain::Voltage,
            },
            inputs: vec![Signal::Block("MOD".to_string())],
        },
    ]
}

fn gates() -> BTreeMap<String, GateBinding> {
    let mut gates = BTreeMap::new();
    gates.insert(
        "D1".to_string(),
        GateBinding::Block("MOD_MAIN_GATE".to_string()),
    );
    gates
}

fn ideal_switches() -> BTreeMap<String, IdealSwitch> {
    let mut m = BTreeMap::new();
    m.insert(
        "D1".to_string(),
        IdealSwitch::new(0.1, IdealDiode::new(0.0, -100.0, 0.0, 0.7, 1.0)),
    );
    m
}

/// Timestamps of every transition (rising or falling) of the `MOD` block's own output.
/// Seeded with the block's own true initial condition (`phase=0`, `red_frac=DEAD_FRAC > 0`, so
/// `main` is OFF *before* any step ever runs) rather than `None` -- otherwise an edge landing
/// exactly on the very first accepted step (as the event clamp itself can legitimately produce)
/// would go uncounted, which is a test-counting artifact, not a missed edge.
fn edge_times(trace: &[TransientWithBlocksStep]) -> Vec<f64> {
    let mut times = Vec::new();
    let mut prev: Option<f64> = Some(0.0);
    for (t, _, outputs) in trace {
        let v = match outputs.get("MOD") {
            Some(SignalValue::Scalar(v)) => *v,
            _ => continue,
        };
        if let Some(p) = prev {
            if (v - p).abs() > 0.5 {
                times.push(*t);
            }
        }
        prev = Some(v);
    }
    times
}

fn count_edges(trace: &[TransientWithBlocksStep]) -> usize {
    edge_times(trace).len()
}

fn run(step: TimeStep) -> Vec<TransientWithBlocksStep> {
    simulate_transient_with_blocks(
        netlist(),
        Dialect::Ngspice,
        &BTreeMap::new(),
        &ideal_switches(),
        &blocks(),
        &gates(),
        0.1,
        None,
        T_FINAL,
        step,
    )
    .unwrap()
}

#[test]
fn adaptive_step_lands_exactly_on_pwm_edges_no_missed_edges() {
    // Fine fixed-step ground truth: 2000 points per switching period, far finer than the
    // dead-time fraction, so every rising/falling threshold is resolved precisely.
    let ground_truth = run(TimeStep::Fixed(PERIOD / 2000.0));
    let expected_edges = count_edges(&ground_truth);
    // Sanity: with duty=0.4 and dead time on both edges, MOD's own {main-on, main-off}
    // transitions happen twice per period (red_frac and duty thresholds) -- confirms the
    // ground truth run itself is behaving as designed, not just "some number of edges."
    assert_eq!(
        expected_edges,
        2 * NUM_PERIODS as usize,
        "ground-truth run's own MOD edge count should be exactly 2 per period"
    );

    // The same dt_max levels (as a fraction of the switching period) that the sibling
    // `elspice-pwl-ps-pwm-leg-modulator` experiment measured as 30/100, 45/100, and 57/100
    // missed edges under the *old*, unclamped adaptive controller.
    for dt_max_frac in [0.48, 0.9, 3.0] {
        let dt_max = dt_max_frac * PERIOD;
        let config = AdaptiveConfig {
            dt_init: PERIOD / 50.0,
            dt_min: PERIOD * 1e-6,
            dt_max,
            reltol: 1e-3,
            abstol: 1e-6,
        };
        let trace = run(TimeStep::Adaptive(config));
        let got_edges = count_edges(&trace);
        if got_edges != expected_edges {
            let gt = edge_times(&ground_truth);
            let ad = edge_times(&trace);
            eprintln!("ground truth edges ({}): {:?}", gt.len(), gt);
            eprintln!("adaptive edges ({}): {:?}", ad.len(), ad);
        }
        assert_eq!(
            got_edges,
            expected_edges,
            "dt_max={dt_max_frac}x period: expected {expected_edges} MOD edges (matching fine \
             fixed-step ground truth), got {got_edges} -- missed {}",
            expected_edges as i64 - got_edges as i64
        );

        // Bounded step count: a real bug (fixed alongside this test -- see
        // `next_pwm_edge_dt`'s own doc comment in block_graph.rs) made landing exactly on an
        // edge report that same edge as "0 dt away" on the very next call, stalling the
        // adaptive loop at a near-zero dt indefinitely -- correct edge COUNT eventually, given
        // enough (potentially billions of) steps, but each stalled step still grows the
        // in-memory trace, which is what actually exhausted memory on a real run. This bound
        // (100x the number of true edges, generous headroom over the ~1-2 steps/edge this event
        // clamp should normally need) catches that stall class directly, not just its eventual
        // correctness.
        assert!(
            trace.len() < 100 * expected_edges,
            "dt_max={dt_max_frac}x period: {} steps for {expected_edges} true edges is far more \
             than the event clamp should ever need -- looks like a near-zero-dt stall, not \
             normal LTE-driven refinement",
            trace.len()
        );
    }
}
