//! `BlockKind::Sig2Phys` (`domain=voltage`/`domain=current`) — the enforced Signal-to-PS boundary
//! for a block driving an independent source's own magnitude, closing the write-direction gap
//! `BlockKind::Phys2Sig` alone leaves open (a block could previously only ever observe a circuit,
//! never load or drive it — see `elspice-pwl-buck-dc-motor-cascade`'s own documented
//! limitation). Verifies both the enforcement (a `V`/`I` source's bare-symbol literal must name
//! a `Sig2Phys` converter of the matching domain, not any other block, `V` needs
//! `domain=voltage`/`I` needs `domain=current`) and the actual numeric substitution, against
//! exact hand-derived values: first
//! a pure resistive circuit with no reactive elements, so the circuit's own response is an
//! *identity*, not an approximation — the cleanest possible check that the source's magnitude
//! genuinely comes from the block graph's own output every step; then a genuine RC circuit
//! specifically to exercise `Scheme::Trapezoidal`'s own `u_prev` history-averaging path for a
//! block-driven source (the resistive-only cases have `K = 0`, so trapezoidal degenerates to
//! the same algebraic solve as `Scheme::Dc` and never actually exercises that averaging term).

use std::collections::BTreeMap;

use dae_runtime::{
    simulate_transient_with_blocks, BlockInstance, BlockKind, ConstValue, DaeError, PhysicalDomain,
    Signal, TimeStep,
};
use general_spice_core::Dialect;
use pwl_devices::{IdealDiode, IdealSwitch};

#[test]
fn sig2phys_voltage_drives_a_voltage_source_exactly_through_a_pwl_ramp() {
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
            kind: BlockKind::Sig2Phys {
                domain: PhysicalDomain::Voltage,
            },
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
fn sig2phys_current_drives_a_current_source_matching_ohms_law_exactly() {
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
            kind: BlockKind::Sig2Phys {
                domain: PhysicalDomain::Current,
            },
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
fn a_voltage_source_naming_a_domain_current_sig2phys_block_is_rejected() {
    // V1's own literal names CMD_I, which is domain=current -- V needs domain=voltage
    // specifically, so this must be rejected even though CMD_I is a legitimate converter of the
    // *other* domain.
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
            kind: BlockKind::Sig2Phys {
                domain: PhysicalDomain::Current,
            },
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
            assert_eq!(expected_kind, "sig2phys(domain=voltage)");
            assert_eq!(found_kind, "sig2phys(domain=current)");
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
            assert_eq!(expected_kind, "sig2phys(domain=voltage)");
            assert_eq!(found_kind, "const");
        }
        other => panic!("expected SourceNotSig2PhysicalConverter, got {other:?}"),
    }
}

#[test]
fn wiring_a_converter_into_the_netlist_as_a_node_is_rejected_before_any_step_runs() {
    // The silent-zero bug this guards: `CMD_V` is a Sig2Phys converter, which has no terminals
    // and stamps nothing, so the net that happens to share its name was just an ordinary
    // undriven node -- R1 to ground makes `G*v = 0` perfectly non-singular, so the solve
    // *succeeded* and reported V(CMD_V) = 0 next to the block's own correct, non-zero output
    // column. A converter is consumed by name (a V/I source's value field, or a switch's
    // gate=/ctrl=), never by a wire, so this must be a hard build-time error instead.
    let netlist = "R1 CMD_V 0 1000";
    let ideal_switches: BTreeMap<String, IdealSwitch> = BTreeMap::new();
    let diodes: BTreeMap<String, IdealDiode> = BTreeMap::new();

    let blocks = vec![
        BlockInstance {
            name: "CMD".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(5.0)),
            inputs: vec![],
        },
        BlockInstance {
            name: "CMD_V".to_string(),
            kind: BlockKind::Sig2Phys {
                domain: PhysicalDomain::Voltage,
            },
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
        1e-4,
        TimeStep::Fixed(1e-5),
    )
    .unwrap_err();

    match err {
        DaeError::Sig2PhysUsedAsCircuitNode {
            block,
            element,
            node,
        } => {
            assert_eq!(block, "CMD_V");
            assert_eq!(element, "R1");
            assert_eq!(node, "CMD_V");
        }
        other => panic!("expected Sig2PhysUsedAsCircuitNode, got {other:?}"),
    }
}

#[test]
fn wiring_a_converter_as_a_node_is_rejected_case_insensitively() {
    // Node names are one namespace regardless of case in this grammar (`A` and `a` are the same
    // node), so `R1 cmd_v 0` against a converter declared `CMD_V` is the identical mistake and
    // must produce the identical error -- reported with the node token exactly as written.
    let netlist = "V1 a 0 5\nR1 a cmd_v 1000\nR2 cmd_v 0 1000";
    let ideal_switches: BTreeMap<String, IdealSwitch> = BTreeMap::new();
    let diodes: BTreeMap<String, IdealDiode> = BTreeMap::new();

    let blocks = vec![
        BlockInstance {
            name: "CMD".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(5.0)),
            inputs: vec![],
        },
        BlockInstance {
            name: "CMD_V".to_string(),
            kind: BlockKind::Sig2Phys {
                domain: PhysicalDomain::Current,
            },
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
        1e-4,
        TimeStep::Fixed(1e-5),
    )
    .unwrap_err();

    match err {
        DaeError::Sig2PhysUsedAsCircuitNode {
            block,
            element,
            node,
        } => {
            assert_eq!(block, "CMD_V");
            // R1 is the first element wiring it, not R2.
            assert_eq!(element, "R1");
            assert_eq!(node, "cmd_v");
        }
        other => panic!("expected Sig2PhysUsedAsCircuitNode, got {other:?}"),
    }
}

#[test]
fn a_node_merely_named_like_a_non_converter_block_is_still_allowed() {
    // The check is deliberately Sig2Phys-only: naming, say, a phys2sig probe after the node it
    // measures is a plausible habit and must keep working. `PROBE` here is a plain Const, and
    // node `PROBE` is a legitimate, driven node -- nothing to reject.
    let netlist = "V1 PROBE 0 5\nR1 PROBE 0 1000";
    let ideal_switches: BTreeMap<String, IdealSwitch> = BTreeMap::new();
    let diodes: BTreeMap<String, IdealDiode> = BTreeMap::new();

    let blocks = vec![BlockInstance {
        name: "PROBE".to_string(),
        kind: BlockKind::Const(ConstValue::Scalar(1.0)),
        inputs: vec![],
    }];

    let trace = simulate_transient_with_blocks(
        netlist,
        Dialect::Ngspice,
        &diodes,
        &ideal_switches,
        &blocks,
        &BTreeMap::new(),
        0.1,
        None,
        1e-4,
        TimeStep::Fixed(1e-5),
    )
    .unwrap();

    for (_, point, _) in &trace {
        assert!((point.value("V(PROBE)").unwrap() - 5.0).abs() < 1e-9);
    }
}

#[test]
fn sig2phys_voltage_driven_source_produces_correct_rc_charging_dynamics_under_trapezoidal() {
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
            kind: BlockKind::Sig2Phys {
                domain: PhysicalDomain::Voltage,
            },
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
