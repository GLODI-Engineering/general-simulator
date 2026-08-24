//! `Signal::BlockPrev` — reading a named block's own output from the *previous* step, the
//! block-graph counterpart to `BlockKind::Probe`'s "read the circuit's previous state." Added
//! specifically to close a loop *around a block itself* (a controller regulating a `Pmsm`'s own
//! `id`/`iq`, or a PLL's angle feeding the very `Park` block that produced its error), where a
//! same-step self-reference is a genuine algebraic loop no feedforward declaration order can
//! resolve.
//!
//! Simplest possible exercise of exactly that self-referencing pattern: a discrete accumulator
//! built from nothing but `Sum` and its own `prev:` reference (`ACC = prev:ACC + IN` every
//! step, `IN` fixed at `1.0`) — after `n` steps this must equal `n` exactly, a trivial
//! hand-derived closed form, checked against every step's own value (not just the final one).

use std::collections::BTreeMap;

use dae_runtime::{simulate_transient_with_blocks, BlockInstance, BlockKind, Signal, TimeStep};
use general_spice_core::Dialect;
use pwl_devices::{Diode, Mosfet};

#[test]
fn block_prev_self_reference_builds_an_exact_discrete_accumulator() {
    let netlist = "V1 a 0 5\nR1 a 0 1k";
    let mosfets: BTreeMap<String, Mosfet> = BTreeMap::new();
    let diodes: BTreeMap<String, Diode> = BTreeMap::new();
    let gates = BTreeMap::new();

    let blocks = vec![
        BlockInstance {
            name: "IN".to_string(),
            kind: BlockKind::Const(1.0),
            inputs: vec![],
        },
        BlockInstance {
            name: "ACC".to_string(),
            kind: BlockKind::Sum(vec![1.0, 1.0]),
            inputs: vec![
                Signal::BlockPrev("ACC".to_string()),
                Signal::Block("IN".to_string()),
            ],
        },
    ];

    let dt = 1e-6;
    let n_steps = 50;
    let t_final = dt * n_steps as f64;

    let trace = simulate_transient_with_blocks(
        netlist,
        Dialect::Ngspice,
        &diodes,
        &mosfets,
        &blocks,
        &gates,
        0.1,
        None,
        t_final,
        TimeStep::Fixed(dt),
    )
    .unwrap();

    assert_eq!(trace.len(), n_steps);
    for (i, (_, _, outputs)) in trace.iter().enumerate() {
        let expected = (i + 1) as f64; // step 1 -> ACC=1, step 2 -> ACC=2, ...
        assert!(
            (outputs["ACC"] - expected).abs() < 1e-12,
            "step {i}: ACC={}, expected={expected}",
            outputs["ACC"]
        );
    }
}

#[test]
fn block_prev_of_an_unevaluated_block_is_zero_before_the_first_step() {
    // A one-shot check that a `prev:` reference to a name that never otherwise appears in the
    // graph resolves to 0.0 rather than an error -- the documented default, distinct from
    // `Signal::Block`'s behavior (an unknown *same-step* name is a hard error).
    let netlist = "V1 a 0 5\nR1 a 0 1k";
    let mosfets: BTreeMap<String, Mosfet> = BTreeMap::new();
    let diodes: BTreeMap<String, Diode> = BTreeMap::new();
    let gates = BTreeMap::new();

    let blocks = vec![BlockInstance {
        name: "OUT".to_string(),
        kind: BlockKind::Gain(2.0),
        inputs: vec![Signal::BlockPrev("NOWHERE".to_string())],
    }];

    let trace = simulate_transient_with_blocks(
        netlist,
        Dialect::Ngspice,
        &diodes,
        &mosfets,
        &blocks,
        &gates,
        0.1,
        None,
        1e-6,
        TimeStep::Fixed(1e-7),
    )
    .unwrap();

    let (_, _, outputs) = trace.first().unwrap();
    assert_eq!(outputs["OUT"], 0.0);
}
