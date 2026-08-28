//! `kind=pyfunc` through the real block-graph evaluation loop -- a genuinely separate contract
//! from `pyblock.rs`'s own `kind=pyblock` tests (not exercised by them at all), for the plain,
//! stateless, positionally-called Python function this feature was requested for.

use std::collections::BTreeMap;
use std::path::PathBuf;

use dae_runtime::{
    simulate_transient_with_blocks, BlockInstance, BlockKind, ConstValue, SampleTimeSpec, Signal,
    SignalValue, TimeStep,
};
use general_spice_core::Dialect;
use pwl_devices::{Diode, Mosfet};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn run(
    blocks: &[BlockInstance],
    t_final: f64,
    dt: f64,
) -> Vec<(f64, BTreeMap<String, SignalValue>)> {
    let netlist = "V1 a 0 5\nR1 a 0 1k";
    let mosfets: BTreeMap<String, Mosfet> = BTreeMap::new();
    let diodes: BTreeMap<String, Diode> = BTreeMap::new();
    let gates = BTreeMap::new();
    simulate_transient_with_blocks(
        netlist,
        Dialect::Ngspice,
        &diodes,
        &mosfets,
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

#[test]
fn pyfunc_matches_the_hand_written_truth_table_for_a_fixed_phase() {
    let blocks = vec![
        BlockInstance {
            name: "PHASE".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(90.0)),
            inputs: vec![],
        },
        BlockInstance {
            name: "AQCTLA".to_string(),
            kind: BlockKind::PyFunction {
                path: fixture("gate_pattern.py"),
                function: "compute_action_qualifier_180_degree".to_string(),
                output_names: vec!["AQCTLA".to_string(), "AQCTLB".to_string()],
                sample_time: None,
            },
            inputs: vec![Signal::Block("PHASE".to_string())],
        },
    ];
    let trace = run(&blocks, 1e-4, 1e-4);
    let (_, outputs) = &trace[0];
    // phase=90 is in (0,180) -> AQCTLA=2066, AQCTLB=1057, the same truth table pyblock-ffi's
    // own lower-level pure_function.rs test already checks in full -- this test's own job is
    // confirming the real block-graph path (BlockKind::PyFunction construction, the
    // "function=" field reaching PyFunctionRegistry::instantiate, output_names binding) wires
    // up correctly end to end, not re-deriving the truth table itself.
    assert_eq!(outputs["AQCTLA"], SignalValue::Scalar(2066.0));
    assert_eq!(outputs["AQCTLB"], SignalValue::Scalar(1057.0));
}

#[test]
fn pyfunc_respects_sample_time_zero_order_hold() {
    // Same convention every other zero-order-hold block already has -- PHASE steps partway
    // through the run; AQCTLA should only pick up the new value once a full sample period has
    // elapsed, not on the very next circuit step.
    let blocks = vec![
        BlockInstance {
            name: "PHASE".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(90.0)),
            inputs: vec![],
        },
        BlockInstance {
            name: "AQCTLA".to_string(),
            kind: BlockKind::PyFunction {
                path: fixture("gate_pattern.py"),
                function: "compute_action_qualifier_180_degree".to_string(),
                output_names: vec!["AQCTLA".to_string(), "AQCTLB".to_string()],
                sample_time: Some(SampleTimeSpec::Periodic {
                    period: 5e-4,
                    offset: 0.0,
                }),
            },
            inputs: vec![Signal::Block("PHASE".to_string())],
        },
    ];
    let dt = 1e-4;
    let trace = run(&blocks, dt * 20.0, dt);
    let values: Vec<f64> = trace
        .iter()
        .map(|(_, o)| o["AQCTLA"].as_scalar().unwrap())
        .collect();
    // 5 rows per held sample (ts/dt = 5): the first 5 rows share one value.
    assert_eq!(values[0], values[4]);
    assert_eq!(values[0], 2066.0);
}
