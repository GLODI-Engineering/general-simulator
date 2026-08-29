//! Gates/latch/flip-flops/counter through the real block-graph evaluation loop -- confirms
//! edge detection and next-state rules work correctly across a real sequence of resolved
//! transient steps, not just as isolated function calls (already covered by
//! `continuous-blocks`'s own unit tests). Clock/set/reset signals are driven with `kind=pwc`
//! (piecewise-constant), whose own breakpoints are chosen to land exactly on the fixed `dt`
//! grid, so every edge instant is exact, not approximate.

use std::collections::BTreeMap;

use continuous_blocks::{FlipFlopKind, LatchPriority, LogicOp};
use dae_runtime::{
    simulate_transient_with_blocks, BlockInstance, BlockKind, ConstValue, Signal, SignalValue,
    TimeStep,
};
use general_spice_core::Dialect;
use pwl_devices::{IdealDiode, IdealSwitch};

fn run(
    blocks: &[BlockInstance],
    t_final: f64,
    dt: f64,
) -> Vec<(f64, BTreeMap<String, SignalValue>)> {
    let netlist = "V1 a 0 5\nR1 a 0 1k";
    let ideal_switches: BTreeMap<String, IdealSwitch> = BTreeMap::new();
    let diodes: BTreeMap<String, IdealDiode> = BTreeMap::new();
    let gates = BTreeMap::new();
    simulate_transient_with_blocks(
        netlist,
        Dialect::Ngspice,
        &diodes,
        &ideal_switches,
        blocks,
        &gates,
        0.1,
        None,
        t_final,
        TimeStep::Fixed(dt),
    )
    .unwrap()
    .into_iter()
    .map(|(t, _, outputs)| (t, outputs))
    .collect()
}

fn pwc(name: &str, points: &[(f64, f64)]) -> BlockInstance {
    BlockInstance {
        name: name.to_string(),
        kind: BlockKind::Pwc {
            points: points.to_vec(),
            repeat: false,
        },
        inputs: vec![],
    }
}

fn value_at(trace: &[(f64, BTreeMap<String, SignalValue>)], t_target: f64, name: &str) -> f64 {
    trace
        .iter()
        .find(|(t, _)| (*t - t_target).abs() < 1e-9)
        .unwrap_or_else(|| panic!("no row at t={t_target}"))
        .1[name]
        .as_scalar()
        .unwrap()
}

#[test]
fn and_or_xor_not_match_the_truth_table_through_the_real_graph() {
    let blocks = vec![
        BlockInstance {
            name: "A".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(1.0)),
            inputs: vec![],
        },
        BlockInstance {
            name: "B".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(0.0)),
            inputs: vec![],
        },
        BlockInstance {
            name: "AND1".to_string(),
            kind: BlockKind::LogicGate(LogicOp::And),
            inputs: vec![
                Signal::Block("A".to_string()),
                Signal::Block("B".to_string()),
            ],
        },
        BlockInstance {
            name: "OR1".to_string(),
            kind: BlockKind::LogicGate(LogicOp::Or),
            inputs: vec![
                Signal::Block("A".to_string()),
                Signal::Block("B".to_string()),
            ],
        },
        BlockInstance {
            name: "XOR1".to_string(),
            kind: BlockKind::LogicGate(LogicOp::Xor),
            inputs: vec![
                Signal::Block("A".to_string()),
                Signal::Block("B".to_string()),
            ],
        },
        BlockInstance {
            name: "NOT1".to_string(),
            kind: BlockKind::LogicGate(LogicOp::Not),
            inputs: vec![Signal::Block("A".to_string())],
        },
    ];
    let trace = run(&blocks, 1e-4, 1e-4);
    let (_, outputs) = &trace[0];
    // A=1, B=0.
    assert_eq!(outputs["AND1"], SignalValue::Scalar(0.0));
    assert_eq!(outputs["OR1"], SignalValue::Scalar(1.0));
    assert_eq!(outputs["XOR1"], SignalValue::Scalar(1.0));
    assert_eq!(outputs["NOT1"], SignalValue::Scalar(0.0));
}

#[test]
fn srlatch_stays_latched_until_reset_and_set_wins_when_both_asserted() {
    // set pulses high at t=1ms..2ms, reset pulses high at t=3ms..4ms, both high at t=5ms..6ms
    // (checking Set-priority: must stay/become latched true, not clear).
    let set = pwc(
        "SET",
        &[
            (0.0, 0.0),
            (0.001, 1.0),
            (0.002, 0.0),
            (0.005, 1.0),
            (0.006, 0.0),
        ],
    );
    let reset = pwc(
        "RESET",
        &[
            (0.0, 0.0),
            (0.003, 1.0),
            (0.004, 0.0),
            (0.005, 1.0),
            (0.006, 0.0),
        ],
    );
    let latch = BlockInstance {
        name: "FAULT".to_string(),
        kind: BlockKind::SrLatch {
            priority: LatchPriority::Set,
        },
        inputs: vec![
            Signal::Block("SET".to_string()),
            Signal::Block("RESET".to_string()),
        ],
    };
    let trace = run(&[set, reset, latch], 0.007, 0.0005);

    assert_eq!(value_at(&trace, 0.0005, "FAULT"), 0.0); // before set
    assert_eq!(value_at(&trace, 0.0015, "FAULT"), 1.0); // set asserted -> latched
    assert_eq!(value_at(&trace, 0.0025, "FAULT"), 1.0); // set released -> still latched (hold)
    assert_eq!(value_at(&trace, 0.0035, "FAULT"), 0.0); // reset asserted -> cleared
    assert_eq!(value_at(&trace, 0.0045, "FAULT"), 0.0); // reset released -> still cleared
    assert_eq!(value_at(&trace, 0.0055, "FAULT"), 1.0); // both asserted -> set wins
}

#[test]
fn dff_only_updates_q_on_a_rising_clk_edge() {
    // clk rises at t=1ms and t=3ms; d changes to 1 at t=0.5ms (before the first edge, so Q
    // should capture 1 at the first edge) and back to 0 at t=2ms (before the second edge, so Q
    // should capture 0 at the second edge).
    let clk = pwc(
        "CLK",
        &[
            (0.0, 0.0),
            (0.001, 1.0),
            (0.0015, 0.0),
            (0.003, 1.0),
            (0.0035, 0.0),
        ],
    );
    let d = pwc("D", &[(0.0, 0.0), (0.0005, 1.0), (0.002, 0.0)]);
    let dff = BlockInstance {
        name: "Q".to_string(),
        kind: BlockKind::FlipFlop {
            kind: FlipFlopKind::D,
            reset: false,
        },
        inputs: vec![
            Signal::Block("CLK".to_string()),
            Signal::Block("D".to_string()),
        ],
    };
    let trace = run(&[clk, d, dff], 0.004, 0.0001);

    assert_eq!(value_at(&trace, 0.0009, "Q"), 0.0); // before the first edge
    assert_eq!(value_at(&trace, 0.0011, "Q"), 1.0); // captured d=1 at the first rising edge
    assert_eq!(value_at(&trace, 0.0025, "Q"), 1.0); // holds between edges, even though d fell
    assert_eq!(value_at(&trace, 0.0031, "Q"), 0.0); // captured d=0 at the second rising edge
}

#[test]
fn tff_toggles_once_per_rising_edge_a_classic_divide_by_two() {
    let clk = pwc(
        "CLK",
        &[
            (0.0, 0.0),
            (0.001, 1.0),
            (0.0015, 0.0),
            (0.002, 1.0),
            (0.0025, 0.0),
            (0.003, 1.0),
            (0.0035, 0.0),
        ],
    );
    let t_in = BlockInstance {
        name: "T".to_string(),
        kind: BlockKind::Const(ConstValue::Scalar(1.0)),
        inputs: vec![],
    };
    let tff = BlockInstance {
        name: "Q".to_string(),
        kind: BlockKind::FlipFlop {
            kind: FlipFlopKind::T,
            reset: false,
        },
        inputs: vec![
            Signal::Block("CLK".to_string()),
            Signal::Block("T".to_string()),
        ],
    };
    let trace = run(&[clk, t_in, tff], 0.004, 0.0001);

    // Three rising edges (t=0.001, 0.002, 0.003) -> toggles 0->1->0->1.
    assert_eq!(value_at(&trace, 0.0005, "Q"), 0.0);
    assert_eq!(value_at(&trace, 0.0011, "Q"), 1.0);
    assert_eq!(value_at(&trace, 0.0021, "Q"), 0.0);
    assert_eq!(value_at(&trace, 0.0031, "Q"), 1.0);
}

#[test]
fn counter_increments_once_per_edge_and_wraps_at_the_declared_modulus() {
    // 5 rising edges at t=1,2,3,4,5 ms; modulus=3 -> 1,2,0,1,2.
    let clk = pwc(
        "CLK",
        &[
            (0.0, 0.0),
            (0.001, 1.0),
            (0.0015, 0.0),
            (0.002, 1.0),
            (0.0025, 0.0),
            (0.003, 1.0),
            (0.0035, 0.0),
            (0.004, 1.0),
            (0.0045, 0.0),
            (0.005, 1.0),
            (0.0055, 0.0),
        ],
    );
    let counter = BlockInstance {
        name: "CNT".to_string(),
        kind: BlockKind::Counter {
            up_down: false,
            modulus: Some(3),
            reset: false,
        },
        inputs: vec![Signal::Block("CLK".to_string())],
    };
    let trace = run(&[clk, counter], 0.006, 0.0001);

    assert_eq!(value_at(&trace, 0.0011, "CNT"), 1.0);
    assert_eq!(value_at(&trace, 0.0021, "CNT"), 2.0);
    assert_eq!(value_at(&trace, 0.0031, "CNT"), 0.0); // wraps 3 -> 0
    assert_eq!(value_at(&trace, 0.0041, "CNT"), 1.0);
    assert_eq!(value_at(&trace, 0.0051, "CNT"), 2.0);
}
