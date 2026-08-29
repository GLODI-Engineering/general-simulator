//! `GateBinding` targets must resolve to a `BlockKind::Sig2Voltage` converter, never a raw
//! control block directly — an ideal switch's gate is itself a voltage, so it shares the same
//! Signal-to-PS boundary a V-source's own magnitude uses; no separate gate-only converter
//! exists. Checks both that the wrong kind is rejected before any step runs
//! (`DaeError::GateTargetNotSig2Voltage`, naming the actual offending block and kind) and that
//! the correct kind (wrapped in `Sig2Voltage`) works exactly as the pre-enforcement direct
//! reference used to.

use std::collections::BTreeMap;

use dae_runtime::{
    simulate_transient_with_blocks, BlockInstance, BlockKind, ConstValue, DaeError, GateBinding,
    Signal, TimeStep,
};
use general_spice_core::Dialect;
use pwl_devices::{IdealDiode, IdealSwitch};

fn dummy_ideal_switches() -> BTreeMap<String, IdealSwitch> {
    let mut m = BTreeMap::new();
    m.insert(
        "D1".to_string(),
        IdealSwitch::new(0.1, IdealDiode::new(0.0, -1e6, 1e-6, 1e6, 0.0)),
    );
    m
}

#[test]
fn naming_a_raw_block_directly_is_rejected_before_any_step_runs() {
    let netlist = "V1 a 0 5\nD1 a b idealswitchmodel\nR1 b 0 1000";
    let ideal_switches = dummy_ideal_switches();
    let diodes = BTreeMap::new();

    let blocks = vec![BlockInstance {
        name: "DUTY".to_string(),
        kind: BlockKind::Const(ConstValue::Scalar(0.3)),
        inputs: vec![],
    }];
    let mut gates = BTreeMap::new();
    gates.insert(
        "D1".to_string(),
        // raw Const, not wrapped in Sig2Voltage -- must be rejected
        GateBinding::Block("DUTY".to_string()),
    );

    let err = simulate_transient_with_blocks(
        netlist,
        Dialect::Ngspice,
        &diodes,
        &ideal_switches,
        &blocks,
        &gates,
        0.1,
        None,
        1e-5,
        TimeStep::Fixed(1e-6),
    )
    .unwrap_err();

    match err {
        DaeError::GateTargetNotSig2Voltage {
            gate,
            block,
            found_kind,
        } => {
            assert_eq!(gate, "D1");
            assert_eq!(block, "DUTY");
            assert_eq!(found_kind, "const");
        }
        other => panic!("expected GateTargetNotSig2Voltage, got {other:?}"),
    }
}

#[test]
fn wrapping_the_same_block_in_sig2voltage_makes_it_work() {
    let netlist = "V1 a 0 5\nD1 a b idealswitchmodel\nR1 b 0 1000";
    let ideal_switches = dummy_ideal_switches();
    let diodes = BTreeMap::new();

    let blocks = vec![
        BlockInstance {
            name: "DUTY".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(0.3)),
            inputs: vec![],
        },
        BlockInstance {
            name: "DUTY_GATE".to_string(),
            kind: BlockKind::Sig2Voltage,
            inputs: vec![Signal::Block("DUTY".to_string())],
        },
    ];
    let mut gates = BTreeMap::new();
    gates.insert(
        "D1".to_string(),
        GateBinding::Block("DUTY_GATE".to_string()),
    );

    // Should simply succeed -- correctness of the resulting switching is already covered by
    // pwm_complement_gate.rs; this test is specifically about the enforcement gate passing.
    let trace = simulate_transient_with_blocks(
        netlist,
        Dialect::Ngspice,
        &diodes,
        &ideal_switches,
        &blocks,
        &gates,
        0.1,
        None,
        1e-5,
        TimeStep::Fixed(1e-6),
    )
    .unwrap();
    assert!(!trace.is_empty());
}
