//! `BlockKind::Sig2Voltage`/`BlockKind::Sig2Current` — the enforced Signal-to-PS boundary for a
//! block driving an independent source's own magnitude, closing the write-direction gap
//! `BlockKind::Probe` alone leaves open (a block could previously only ever observe a circuit,
//! never load or drive it — see `elspice-pwl-buck-dc-motor-cascade`'s own documented
//! limitation). Verifies both the enforcement (a `V`/`I` source's bare-symbol literal must name
//! the matching converter kind, not any other block, `V` needs `Sig2Voltage`/`I` needs
//! `Sig2Current`) and the actual numeric substitution, against exact hand-derived values: first
//! a pure resistive circuit with no reactive elements, so the circuit's own response is an
//! *identity*, not an approximation — the cleanest possible check that the source's magnitude
//! genuinely comes from the block graph's own output every step; then a genuine RC circuit
//! specifically to exercise `Scheme::Trapezoidal`'s own `u_prev` history-averaging path for a
//! block-driven source (the resistive-only cases have `K = 0`, so trapezoidal degenerates to
//! the same algebraic solve as `Scheme::Dc` and never actually exercises that averaging term).

use std::collections::BTreeMap;

use dae_runtime::{
    simulate_transient_with_blocks, BlockInstance, BlockKind, ConstValue, DaeError, Signal,
    TimeStep,
};
use general_spice_core::Dialect;
use pwl_devices::{IdealDiode, IdealSwitch};

#[test]
fn sig2voltage_drives_a_voltage_source_exactly_through_a_pwl_ramp() {
    // V1's own literal names CMD_V directly -- general-mna already accepts a bare symbol there
    // (Expression::Symbol), confirmed to need no change on that side.
    let netlist = "V1 a 0 CMD_V\nR1 a 0 1000";
    let ideal_switches: BTreeMap<String, IdealSwitch> = BTreeMap::new();
    let diodes: BTreeMap<String, IdealDiode> = BTreeMap::new();
    let gates = BTreeMap::new();

    let blocks = vec![
        BlockInstance {
            name: "CMD".to_string(),
            kind: BlockKind::Pwc {
                points: vec![(0.0, 2.0), (5e-4, 8.0)],
                repeat: false,
            },
            inputs: vec![],
        },
        BlockInstance {
            name: "CMD_V".to_string(),
            kind: BlockKind::Sig2Voltage,
            inputs: vec![Signal::Block("CMD".to_string())],
        },
    ];

    let dt = 1e-4;
    let trace = simulate_transient_with_blocks(
        netlist,
        Dialect::Ngspice,
        &diodes,
        &ideal_switches,
        &blocks,
        &gates,
        0.1,
        None,
        1e-3,
        TimeStep::Fixed(dt),
    )
    .unwrap();

    // A pure resistive circuit with no storage element: V(a) must equal the source's own
    // commanded value at that exact step *exactly* (not approximately) -- a resistor divider
    // with an ideal voltage source has no dynamics to introduce any lag or error, so this is a
    // genuine identity check, not a tolerance-bounded one.
    for (t, point, _) in &trace {
        let expected = if *t < 5e-4 { 2.0 } else { 8.0 };
        let got = point.value("V(a)").unwrap();
        assert!(
            (got - expected).abs() < 1e-9,
            "t={t}: V(a)={got}, expected exactly {expected} (source driven by CMD_V)"
        );
    }
}

#[test]
fn sig2current_drives_a_current_source_matching_ohms_law_exactly() {
    let netlist = "I1 0 a CMD_I\nR1 a 0 1000";
    let ideal_switches: BTreeMap<String, IdealSwitch> = BTreeMap::new();
    let diodes: BTreeMap<String, IdealDiode> = BTreeMap::new();
    let gates = BTreeMap::new();

    let blocks = vec![
        BlockInstance {
            name: "CMD".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(0.01)), // 10mA
            inputs: vec![],
        },
        BlockInstance {
            name: "CMD_I".to_string(),
            kind: BlockKind::Sig2Current,
            inputs: vec![Signal::Block("CMD".to_string())],
        },
    ];

    let trace = simulate_transient_with_blocks(
        netlist,
        Dialect::Ngspice,
        &diodes,
        &ideal_switches,
        &blocks,
        &gates,
        0.1,
        None,
        1e-4,
        TimeStep::Fixed(1e-5),
    )
    .unwrap();

    // Ohm's law: V(a) = I * R = 0.01 * 1000 = 10V, exactly, every step.
    for (t, point, _) in &trace {
        let got = point.value("V(a)").unwrap();
        assert!(
            (got - 10.0).abs() < 1e-9,
            "t={t}: V(a)={got}, expected exactly 10.0 (I=10mA into R1=1k)"
        );
    }
}

#[test]
fn a_voltage_source_naming_a_sig2current_block_is_rejected() {
    // V1's own literal names CMD_I, which is Sig2Current -- V needs Sig2Voltage specifically,
    // so this must be rejected even though CMD_I is a legitimate converter of the *other* kind.
    let netlist = "V1 a 0 CMD_I\nR1 a 0 1000";
    let ideal_switches: BTreeMap<String, IdealSwitch> = BTreeMap::new();
    let diodes: BTreeMap<String, IdealDiode> = BTreeMap::new();

    let blocks = vec![
        BlockInstance {
            name: "CMD".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(5.0)),
            inputs: vec![],
        },
        BlockInstance {
            name: "CMD_I".to_string(),
            kind: BlockKind::Sig2Current,
            inputs: vec![Signal::Block("CMD".to_string())],
        },
    ];

    let err = simulate_transient_with_blocks(
        netlist,
        Dialect::Ngspice,
        &diodes,
        &ideal_switches,
        &blocks,
        &BTreeMap::new(),
        0.1,
        None,
        1e-5,
        TimeStep::Fixed(1e-6),
    )
    .unwrap_err();

    match err {
        DaeError::SourceNotSig2PhysicalConverter {
            source,
            block,
            expected_kind,
            found_kind,
        } => {
            assert_eq!(source, "V1");
            assert_eq!(block, "CMD_I");
            assert_eq!(expected_kind, "sig2voltage");
            assert_eq!(found_kind, "sig2current");
        }
        other => panic!("expected SourceNotSig2PhysicalConverter, got {other:?}"),
    }
}

#[test]
fn a_voltage_source_naming_a_plain_block_directly_is_rejected() {
    let netlist = "V1 a 0 CMD\nR1 a 0 1000";
    let ideal_switches: BTreeMap<String, IdealSwitch> = BTreeMap::new();
    let diodes: BTreeMap<String, IdealDiode> = BTreeMap::new();

    let blocks = vec![BlockInstance {
        name: "CMD".to_string(),
        kind: BlockKind::Const(ConstValue::Scalar(5.0)),
        inputs: vec![],
    }];

    let err = simulate_transient_with_blocks(
        netlist,
        Dialect::Ngspice,
        &diodes,
        &ideal_switches,
        &blocks,
        &BTreeMap::new(),
        0.1,
        None,
        1e-5,
        TimeStep::Fixed(1e-6),
    )
    .unwrap_err();

    match err {
        DaeError::SourceNotSig2PhysicalConverter {
            source,
            block,
            expected_kind,
            found_kind,
        } => {
            assert_eq!(source, "V1");
            assert_eq!(block, "CMD");
            assert_eq!(expected_kind, "sig2voltage");
            assert_eq!(found_kind, "const");
        }
        other => panic!("expected SourceNotSig2PhysicalConverter, got {other:?}"),
    }
}

#[test]
fn sig2voltage_driven_source_produces_correct_rc_charging_dynamics_under_trapezoidal() {
    // R1=1k, C1=1uF -> tau=1ms. CMD steps 0V -> 10V at t=0 (a plain PWL, not a fixed literal),
    // driving V1 through CMD_V -- unlike the two resistive-only tests above, C1 gives this
    // circuit a real K != 0, so Scheme::Trapezoidal's own u_prev history-averaging path is
    // genuinely exercised (not degenerated to the Dc case), and the closed-form
    // Vout=Vin*(1-exp(-t/RC)) result is a real check on it, not a trivial identity.
    let netlist = "V1 a 0 CMD_V\nR1 a vout 1000\nC1 vout 0 1e-6";
    let ideal_switches: BTreeMap<String, IdealSwitch> = BTreeMap::new();
    let diodes: BTreeMap<String, IdealDiode> = BTreeMap::new();
    let gates = BTreeMap::new();

    let blocks = vec![
        BlockInstance {
            name: "CMD".to_string(),
            kind: BlockKind::Pwc {
                points: vec![(0.0, 10.0)],
                repeat: false,
            },
            inputs: vec![],
        },
        BlockInstance {
            name: "CMD_V".to_string(),
            kind: BlockKind::Sig2Voltage,
            inputs: vec![Signal::Block("CMD".to_string())],
        },
    ];

    let tau = 1000.0 * 1e-6;
    let dt = tau / 200.0;
    let t_final = tau * 3.0;

    let trace = simulate_transient_with_blocks(
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
    .unwrap();

    let mut max_err = 0.0_f64;
    for (t, point, _) in &trace {
        let expected = 10.0 * (1.0 - (-t / tau).exp());
        let got = point.value("V(vout)").unwrap();
        max_err = max_err.max((got - expected).abs());
    }
    assert!(
        max_err < 0.02,
        "V(vout) deviates from the hand-derived RC charging curve by {max_err} \
         (source driven by CMD_V through Scheme::Trapezoidal)"
    );
}
