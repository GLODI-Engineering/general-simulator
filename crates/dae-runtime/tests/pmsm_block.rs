//! `BlockKind::Pmsm` wired through the real block graph (not just `continuous-blocks`' own
//! standalone unit tests) — checks the `output_names` convention it shares with `CScript`/
//! `CoordinateTransform`, and reuses the same hand-derived decoupled-R-L-circuit case
//! `continuous_blocks::pmsm`'s own unit tests verify: `vq=0`, `iq(0)=0` keeps `iq` and
//! `omega_m` at exactly zero for all time, leaving `id(t) = (vd/R)*(1 - exp(-t*R/Ld))`, an
//! independent closed-form result. The circuit itself is irrelevant (a bare resistor across a
//! fixed source) — this test is about the block graph, not circuit behavior.

use std::collections::BTreeMap;

use dae_runtime::{
    simulate_transient_with_blocks, BlockInstance, BlockKind, ConstValue, Signal, TimeStep,
};
use general_spice_core::Dialect;
use pwl_devices::{IdealDiode, IdealSwitch};

#[test]
fn pmsm_block_matches_hand_derived_rl_circuit_when_decoupled() {
    let netlist = "V1 a 0 5\nR1 a 0 1k";
    let ideal_switches: BTreeMap<String, IdealSwitch> = BTreeMap::new();
    let diodes: BTreeMap<String, IdealDiode> = BTreeMap::new();
    let gates = BTreeMap::new();

    let (r, l, vd) = (2.0, 5e-3, 10.0);
    let pmsm = continuous_blocks::Pmsm::new(r, l, l, 0.05, 4.0, 1e-4, 0.0).unwrap();

    let blocks = vec![
        BlockInstance {
            name: "VD".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(vd)),
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "VQ".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(0.0)),
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "TLOAD".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(0.0)),
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "M1".to_string(),
            kind: BlockKind::Pmsm {
                pmsm,
                output_names: vec![
                    "M1".to_string(),
                    "M1_iq".to_string(),
                    "M1_omega".to_string(),
                    "M1_theta".to_string(),
                ],
            },
            inputs: vec![
                Signal::Block("VD".to_string()),
                Signal::Block("VQ".to_string()),
                Signal::Block("TLOAD".to_string()),
            ],
            ic: None,
        },
    ];

    let tau = l / r;
    let dt = tau / 200.0;
    let steps = 400;
    let t_final = dt * steps as f64;

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

    let (t_last, _, outputs) = trace.last().unwrap();
    let expected_id = (vd / r) * (1.0 - (-t_last * r / l).exp());

    let tol = 1e-6;
    assert!(
        (outputs["M1"].as_scalar().unwrap() - expected_id).abs() < tol,
        "id={}, expected={}",
        outputs["M1"].as_scalar().unwrap(),
        expected_id
    );
    assert!(
        outputs["M1_iq"].as_scalar().unwrap().abs() < tol,
        "iq={}",
        outputs["M1_iq"].as_scalar().unwrap()
    );
    assert!(
        outputs["M1_omega"].as_scalar().unwrap().abs() < tol,
        "omega_m={}",
        outputs["M1_omega"].as_scalar().unwrap()
    );
    assert!(
        outputs["M1_theta"].as_scalar().unwrap().abs() < tol,
        "theta_e={}",
        outputs["M1_theta"].as_scalar().unwrap()
    );
}
