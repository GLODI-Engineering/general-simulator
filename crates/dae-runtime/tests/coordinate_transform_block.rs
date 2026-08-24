//! `BlockKind::CoordinateTransform` wired through the real block graph (not just
//! `continuous-blocks`' own standalone unit tests) — checks that the `output_names` convention
//! it shares with `BlockKind::CScript` actually works end to end: the block's own name reads
//! the primary output, and the extra `output_names` entries land in the step trace's
//! `block_outputs` map under their own names, resolvable via `Signal::Block` by any downstream
//! block exactly like a first-class one. The circuit itself is irrelevant here (a bare resistor
//! across a fixed source) — this test is about the block graph, not circuit behavior.

use std::collections::BTreeMap;

use dae_runtime::{simulate_transient_with_blocks, BlockInstance, BlockKind, TimeStep};
use general_spice_core::Dialect;
use pwl_devices::{Diode, Mosfet};

#[test]
fn coordinate_transform_block_exposes_all_outputs_by_name() {
    let netlist = "V1 a 0 5\nR1 a 0 1k";
    let mosfets: BTreeMap<String, Mosfet> = BTreeMap::new();
    let diodes: BTreeMap<String, Diode> = BTreeMap::new();
    let gates = BTreeMap::new();

    // Balanced-three-phase-at-theta=0 snapshot: a=1, b=-0.5, c=-0.5, matching
    // continuous-blocks::coordinate_transforms' own hand-derived Clarke test, so the expected
    // outputs (alpha=1, beta=0, zero=0, and d=1, q=0, zero=0 for the fused Clarke-Park at
    // theta=0) are independently known, not just internally self-consistent.
    let blocks = vec![
        BlockInstance {
            name: "A".to_string(),
            kind: BlockKind::Const(1.0),
            inputs: vec![],
        },
        BlockInstance {
            name: "B".to_string(),
            kind: BlockKind::Const(-0.5),
            inputs: vec![],
        },
        BlockInstance {
            name: "C".to_string(),
            kind: BlockKind::Const(-0.5),
            inputs: vec![],
        },
        BlockInstance {
            name: "THETA".to_string(),
            kind: BlockKind::Const(0.0),
            inputs: vec![],
        },
        BlockInstance {
            name: "CLARKE".to_string(),
            kind: BlockKind::CoordinateTransform {
                kind: continuous_blocks::CoordinateTransform::Clarke,
                output_names: vec![
                    "CLARKE".to_string(),
                    "CLARKE_beta".to_string(),
                    "CLARKE_zero".to_string(),
                ],
            },
            inputs: vec![
                dae_runtime::Signal::Block("A".to_string()),
                dae_runtime::Signal::Block("B".to_string()),
                dae_runtime::Signal::Block("C".to_string()),
            ],
        },
        BlockInstance {
            name: "DQ0".to_string(),
            kind: BlockKind::CoordinateTransform {
                kind: continuous_blocks::CoordinateTransform::ClarkePark,
                output_names: vec![
                    "DQ0".to_string(),
                    "DQ0_q".to_string(),
                    "DQ0_zero".to_string(),
                ],
            },
            inputs: vec![
                dae_runtime::Signal::Block("A".to_string()),
                dae_runtime::Signal::Block("B".to_string()),
                dae_runtime::Signal::Block("C".to_string()),
                dae_runtime::Signal::Block("THETA".to_string()),
            ],
        },
    ];

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

    let (_, _, outputs) = trace.last().unwrap();

    let tol = 1e-9;
    assert!(
        (outputs["CLARKE"] - 1.0).abs() < tol,
        "alpha (primary output)"
    );
    assert!(outputs["CLARKE_beta"].abs() < tol, "beta");
    assert!(outputs["CLARKE_zero"].abs() < tol, "zero");

    assert!((outputs["DQ0"] - 1.0).abs() < tol, "d (primary output)");
    assert!(outputs["DQ0_q"].abs() < tol, "q");
    assert!(outputs["DQ0_zero"].abs() < tol, "zero (fused Clarke-Park)");
}
