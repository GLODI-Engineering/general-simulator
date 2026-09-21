//! `GateBinding` targets must resolve to a `domain=voltage` `BlockKind::Sig2Phys` converter,
//! never a raw control block directly and never a `domain=current` `Sig2Phys` either — an ideal
//! switch's gate is itself a voltage, so it shares the same Signal-to-PS boundary a V-source's
//! own magnitude uses; no separate gate-only converter exists. Checks that the wrong kind is
//! rejected before any step runs (`DaeError::GateTargetNotSig2Voltage`, naming the actual
//! offending block and kind) for both a raw control block and a `domain=current` `Sig2Phys`, and
//! that the correct kind (wrapped in a `domain=voltage` `Sig2Phys`) works exactly as the
//! pre-enforcement direct reference used to.

use std::collections::BTreeMap;

use dae_runtime::{
    simulate_transient_with_blocks, BlockInstance, BlockKind, ConstValue, DaeError, GateBinding,
    PhysicalDomain, Signal, TimeStep,
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
        ic: None,
    }];
    let mut gates = BTreeMap::new();
    gates.insert(
        "D1".to_string(),
        // raw Const, not wrapped in a domain=voltage Sig2Phys -- must be rejected
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
fn naming_a_domain_current_sig2phys_block_is_also_rejected() {
    // The load-bearing asymmetry: a gate must specifically be domain=voltage. A domain=current
    // Sig2Phys is a legitimate converter of the *other* domain, and must still be rejected here.
    let netlist = "V1 a 0 5\nD1 a b idealswitchmodel\nR1 b 0 1000";
    let ideal_switches = dummy_ideal_switches();
    let diodes = BTreeMap::new();

    let blocks = vec![
        BlockInstance {
            name: "DUTY".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(0.3)),
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "DUTY_GATE".to_string(),
            kind: BlockKind::Sig2Phys {
                domain: PhysicalDomain::Current,
            },
            inputs: vec![Signal::Block("DUTY".to_string())],
            ic: None,
        },
    ];
    let mut gates = BTreeMap::new();
    gates.insert(
        "D1".to_string(),
        GateBinding::Block("DUTY_GATE".to_string()),
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
            assert_eq!(block, "DUTY_GATE");
            assert_eq!(found_kind, "sig2phys(domain=current)");
        }
        other => panic!("expected GateTargetNotSig2Voltage, got {other:?}"),
    }
}

#[test]
fn wrapping_the_same_block_in_a_domain_voltage_sig2phys_makes_it_work() {
    let netlist = "V1 a 0 5\nD1 a b idealswitchmodel\nR1 b 0 1000";
    let ideal_switches = dummy_ideal_switches();
    let diodes = BTreeMap::new();

    let blocks = vec![
        BlockInstance {
            name: "DUTY".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(0.3)),
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "DUTY_GATE".to_string(),
            kind: BlockKind::Sig2Phys {
                domain: PhysicalDomain::Voltage,
            },
            inputs: vec![Signal::Block("DUTY".to_string())],
            ic: None,
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
