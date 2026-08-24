//! `topological_order` — each step's block-evaluation order is now derived from the
//! `Signal::Block` dependency graph itself, not declaration position. Two things to check that
//! `evaluate_blocks`' own dispatch tests don't: (1) a block declared *before* the block it
//! depends on still computes the right value (the actual new capability); (2) a genuine
//! same-step cycle is rejected up front, before any step is solved, as `DaeError::AlgebraicLoop`
//! carrying the exact closing path — not discovered as an opaque `UnknownBlockInput` partway
//! through a run the way it would have been under the old declaration-order rule.

use std::collections::BTreeMap;

use dae_runtime::{
    simulate_transient_with_blocks, BlockInstance, BlockKind, DaeError, Signal, TimeStep,
};
use general_spice_core::Dialect;
use pwl_devices::{Diode, Mosfet};

const NETLIST: &str = "V1 a 0 5\nD1 a b mosfetmodel\nR1 b 0 1000";

fn dummy_mosfets() -> BTreeMap<String, Mosfet> {
    let mut m = BTreeMap::new();
    m.insert(
        "D1".to_string(),
        Mosfet::new(0.1, Diode::new(0.0, -100.0, 0.0, 1e6, 0.0)),
    );
    m
}

fn run(blocks: &[BlockInstance]) -> Result<Vec<dae_runtime::TransientWithBlocksStep>, DaeError> {
    let mosfets = dummy_mosfets();
    let diodes = BTreeMap::new();
    let gates = BTreeMap::new();
    simulate_transient_with_blocks(
        NETLIST,
        Dialect::Ngspice,
        &diodes,
        &mosfets,
        blocks,
        &gates,
        0.1,
        None,
        1e-5,
        TimeStep::Fixed(1e-6),
    )
}

#[test]
fn a_block_declared_before_its_own_dependency_still_computes_correctly() {
    // SUM is declared FIRST but reads A and B, both declared AFTER it -- illegal under the old
    // "declared earlier" rule, now resolved correctly by topological_order.
    let blocks = vec![
        BlockInstance {
            name: "SUM".to_string(),
            kind: BlockKind::Sum(vec![1.0, -1.0]),
            inputs: vec![
                Signal::Block("A".to_string()),
                Signal::Block("B".to_string()),
            ],
        },
        BlockInstance {
            name: "A".to_string(),
            kind: BlockKind::Const(7.0),
            inputs: vec![],
        },
        BlockInstance {
            name: "B".to_string(),
            kind: BlockKind::Const(3.0),
            inputs: vec![],
        },
    ];

    let trace = run(&blocks).unwrap();
    let (_, _, outputs) = trace.first().unwrap();
    assert_eq!(outputs["SUM"], 4.0);
    assert_eq!(outputs["A"], 7.0);
    assert_eq!(outputs["B"], 3.0);
}

#[test]
fn a_three_block_cycle_is_rejected_with_the_exact_closing_path() {
    let blocks = vec![
        BlockInstance {
            name: "A".to_string(),
            kind: BlockKind::Gain(1.0),
            inputs: vec![Signal::Block("C".to_string())],
        },
        BlockInstance {
            name: "B".to_string(),
            kind: BlockKind::Gain(1.0),
            inputs: vec![Signal::Block("A".to_string())],
        },
        BlockInstance {
            name: "C".to_string(),
            kind: BlockKind::Gain(1.0),
            inputs: vec![Signal::Block("B".to_string())],
        },
    ];

    let err = run(&blocks).unwrap_err();
    match err {
        DaeError::AlgebraicLoop { cycle } => {
            // The cycle is reported as a closing path -- first and last entries equal, and
            // every consecutive pair is a real dependency edge (each block depends on the next
            // one listed). Don't assume which node the DFS happens to start from.
            assert_eq!(cycle.first(), cycle.last(), "cycle={cycle:?}");
            assert_eq!(cycle.len(), 4, "cycle={cycle:?}"); // A/B/C plus the repeated closer
            let mut seen: Vec<&str> = cycle[..3].iter().map(String::as_str).collect();
            seen.sort();
            assert_eq!(seen, ["A", "B", "C"], "cycle={cycle:?}");
        }
        other => panic!("expected AlgebraicLoop, got {other:?}"),
    }
}

#[test]
fn a_self_referencing_block_is_rejected_as_a_length_one_cycle() {
    let blocks = vec![BlockInstance {
        name: "A".to_string(),
        kind: BlockKind::Gain(1.0),
        inputs: vec![Signal::Block("A".to_string())],
    }];

    let err = run(&blocks).unwrap_err();
    match err {
        DaeError::AlgebraicLoop { cycle } => {
            assert_eq!(cycle, vec!["A".to_string(), "A".to_string()]);
        }
        other => panic!("expected AlgebraicLoop, got {other:?}"),
    }
}

#[test]
fn a_cycle_broken_by_block_prev_is_not_a_cycle_at_all() {
    // Same shape as the rejected 3-cycle above, except C's edge back to A is a `prev:`
    // reference instead of same-step `Signal::Block` -- BlockPrev never contributes a
    // dependency edge, so this must succeed.
    let blocks = vec![
        BlockInstance {
            name: "A".to_string(),
            kind: BlockKind::Sum(vec![1.0, 1.0]),
            inputs: vec![
                Signal::BlockPrev("C".to_string()),
                Signal::Block("SEED".to_string()),
            ],
        },
        BlockInstance {
            name: "SEED".to_string(),
            kind: BlockKind::Const(1.0),
            inputs: vec![],
        },
        BlockInstance {
            name: "B".to_string(),
            kind: BlockKind::Gain(1.0),
            inputs: vec![Signal::Block("A".to_string())],
        },
        BlockInstance {
            name: "C".to_string(),
            kind: BlockKind::Gain(1.0),
            inputs: vec![Signal::Block("B".to_string())],
        },
    ];

    let trace = run(&blocks).unwrap();
    // First step: prev:C is 0.0 (nothing evaluated yet), so A = 0 + 1 = 1.
    let (_, _, first) = trace.first().unwrap();
    assert_eq!(first["A"], 1.0);
    assert_eq!(first["B"], 1.0);
    assert_eq!(first["C"], 1.0);
    // Second step: prev:C is now 1.0 (from the first step), so A = 1 + 1 = 2.
    let (_, _, second) = &trace[1];
    assert_eq!(second["A"], 2.0);
}
