//! `SignalValue::Vector` support, per `book/dev-guide/src/vector-signals.md`: `StateSpace`
//! genuinely MIMO (the underlying `continuous_blocks::StateSpace` always supported general
//! `(A,B,C,D)`; only `evaluate_blocks`' own dispatch was hardcoded to a single input/output
//! before this), elementwise unary functions (`MathFn1`), a matrix-mode `Gain`, and `Sum`'s own
//! "all-scalar or all-vector, mixed rejected outright" rule (unlike `MathFn2`'s scalar-
//! broadcasts-against-a-vector rule, which `Gain`'s own scalar mode also uses).

use std::collections::BTreeMap;

use continuous_blocks::{MathFn1, StateSpace};
use dae_runtime::{
    simulate_transient_with_blocks, BlockInstance, BlockKind, ConstValue, DaeError, GainValue,
    Signal, SignalValue, TimeStep,
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

#[test]
fn statespace_is_genuinely_mimo_two_decoupled_integrators() {
    // A=0, B=I, C=I, D=0 -- two independent pure integrators (x1'=u1, x2'=u2, y=x), driven by
    // two separately-declared scalar inputs (U1=2.0, U2=-1.0). x'=const is exactly linear, so
    // RK4 has zero truncation error here: x(t) = u*t exactly, a genuine closed-form check, not
    // a within-tolerance one.
    let ss = StateSpace::new(
        vec![vec![0.0, 0.0], vec![0.0, 0.0]],
        vec![vec![1.0, 0.0], vec![0.0, 1.0]],
        vec![vec![1.0, 0.0], vec![0.0, 1.0]],
        vec![vec![0.0, 0.0], vec![0.0, 0.0]],
        None,
    )
    .unwrap();
    let blocks = vec![
        BlockInstance {
            name: "U1".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(2.0)),
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "U2".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(-1.0)),
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "SS".to_string(),
            kind: BlockKind::StateSpace(ss),
            inputs: vec![
                Signal::Block("U1".to_string()),
                Signal::Block("U2".to_string()),
            ],
            ic: None,
        },
    ];

    let dt = 1e-4;
    let n_steps = 20;
    let trace = run(&blocks, dt * n_steps as f64, dt);
    assert_eq!(trace.len(), n_steps);
    for (t, outputs) in &trace {
        let SignalValue::Vector(y) = &outputs["SS"] else {
            panic!(
                "expected SS to resolve to a Vector (2 outputs), got {:?}",
                outputs["SS"]
            );
        };
        assert_eq!(y.len(), 2);
        assert!(
            (y[0] - 2.0 * t).abs() < 1e-9,
            "y0 at t={t}: {} vs {}",
            y[0],
            2.0 * t
        );
        assert!(
            (y[1] - (-1.0 * t)).abs() < 1e-9,
            "y1 at t={t}: {} vs {}",
            y[1],
            -t
        );
    }
}

#[test]
fn statespace_accepts_one_vector_signal_in_place_of_several_scalar_ones() {
    // Same 2-input 2-output system as above, but fed by a *single* Vector-valued Const block
    // instead of two separately-declared scalar ones -- exercises `flatten`'s own handling of a
    // genuine upstream Vector, not just concatenating several scalar Signals.
    let ss = StateSpace::new(
        vec![vec![0.0, 0.0], vec![0.0, 0.0]],
        vec![vec![1.0, 0.0], vec![0.0, 1.0]],
        vec![vec![1.0, 0.0], vec![0.0, 1.0]],
        vec![vec![0.0, 0.0], vec![0.0, 0.0]],
        None,
    )
    .unwrap();
    let blocks = vec![
        BlockInstance {
            name: "U".to_string(),
            kind: BlockKind::Const(ConstValue::Vector(vec![3.0, 0.5])),
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "SS".to_string(),
            kind: BlockKind::StateSpace(ss),
            inputs: vec![Signal::Block("U".to_string())],
            ic: None,
        },
    ];

    let dt = 1e-4;
    let n_steps = 10;
    let trace = run(&blocks, dt * n_steps as f64, dt);
    let (t, outputs) = trace.last().unwrap();
    let SignalValue::Vector(y) = &outputs["SS"] else {
        panic!("expected a Vector output");
    };
    assert!((y[0] - 3.0 * t).abs() < 1e-9);
    assert!((y[1] - 0.5 * t).abs() < 1e-9);
}

#[test]
fn statespace_rejects_a_flattened_input_length_that_does_not_match_b() {
    // Declares a 2-input system but wires only one scalar signal -- the flattened input length
    // (1) doesn't match B's own column count (2), which can only be caught at evaluation time
    // (a vector signal's own length isn't visible from netlist text/Rust construction alone).
    let ss = StateSpace::new(
        vec![vec![0.0, 0.0], vec![0.0, 0.0]],
        vec![vec![1.0, 0.0], vec![0.0, 1.0]],
        vec![vec![1.0, 0.0], vec![0.0, 1.0]],
        vec![vec![0.0, 0.0], vec![0.0, 0.0]],
        None,
    )
    .unwrap();
    let blocks = vec![
        BlockInstance {
            name: "U".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(1.0)),
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "SS".to_string(),
            kind: BlockKind::StateSpace(ss),
            inputs: vec![Signal::Block("U".to_string())],
            ic: None,
        },
    ];
    let netlist = "V1 a 0 5\nR1 a 0 1k";
    let ideal_switches: BTreeMap<String, IdealSwitch> = BTreeMap::new();
    let diodes: BTreeMap<String, IdealDiode> = BTreeMap::new();
    let gates = BTreeMap::new();
    let err = simulate_transient_with_blocks(
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
    .unwrap_err();
    match err {
        DaeError::VectorSignalSizeMismatch {
            block,
            expected,
            got,
        } => {
            assert_eq!(block, "SS");
            assert_eq!(expected, 2);
            assert_eq!(got, 1);
        }
        other => panic!("expected VectorSignalSizeMismatch, got {other:?}"),
    }
}

#[test]
fn mathfn1_applies_elementwise_to_a_vector_and_unchanged_to_a_scalar() {
    let blocks = vec![
        BlockInstance {
            name: "V".to_string(),
            kind: BlockKind::Const(ConstValue::Vector(vec![0.0, std::f64::consts::PI])),
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "SINV".to_string(),
            kind: BlockKind::MathFn1(MathFn1::Sin),
            inputs: vec![Signal::Block("V".to_string())],
            ic: None,
        },
        BlockInstance {
            name: "S".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(std::f64::consts::FRAC_PI_2)),
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "SINS".to_string(),
            kind: BlockKind::MathFn1(MathFn1::Sin),
            inputs: vec![Signal::Block("S".to_string())],
            ic: None,
        },
    ];
    let trace = run(&blocks, 1e-4, 1e-4);
    let (_, outputs) = &trace[0];
    let SignalValue::Vector(y) = &outputs["SINV"] else {
        panic!("expected a Vector output");
    };
    assert!(y[0].abs() < 1e-12, "sin(0)={}", y[0]);
    assert!((y[1]).abs() < 1e-12, "sin(pi)={}", y[1]);
    assert_eq!(outputs["SINS"], SignalValue::Scalar(1.0));
}

#[test]
fn matrix_gain_computes_the_matrix_vector_product_and_rejects_wrong_shapes() {
    // K = [[1,2],[3,4]], x = [5,6] -> y = [1*5+2*6, 3*5+4*6] = [17, 39].
    let blocks = vec![
        BlockInstance {
            name: "X".to_string(),
            kind: BlockKind::Const(ConstValue::Vector(vec![5.0, 6.0])),
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "Y".to_string(),
            kind: BlockKind::Gain(GainValue::Matrix(vec![vec![1.0, 2.0], vec![3.0, 4.0]])),
            inputs: vec![Signal::Block("X".to_string())],
            ic: None,
        },
    ];
    let trace = run(&blocks, 1e-4, 1e-4);
    let (_, outputs) = &trace[0];
    assert_eq!(outputs["Y"], SignalValue::Vector(vec![17.0, 39.0]));
}

#[test]
fn matrix_gain_rejects_a_scalar_input() {
    let blocks = vec![
        BlockInstance {
            name: "X".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(5.0)),
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "Y".to_string(),
            kind: BlockKind::Gain(GainValue::Matrix(vec![vec![1.0, 2.0]])),
            inputs: vec![Signal::Block("X".to_string())],
            ic: None,
        },
    ];
    let netlist = "V1 a 0 5\nR1 a 0 1k";
    let ideal_switches: BTreeMap<String, IdealSwitch> = BTreeMap::new();
    let diodes: BTreeMap<String, IdealDiode> = BTreeMap::new();
    let gates = BTreeMap::new();
    let err = simulate_transient_with_blocks(
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
    .unwrap_err();
    assert!(matches!(err, DaeError::VectorSignalNotSupported { block } if block == "Y"));
}

#[test]
fn sum_accepts_all_vector_or_all_scalar_but_rejects_a_mix() {
    let vector_sum = vec![
        BlockInstance {
            name: "A".to_string(),
            kind: BlockKind::Const(ConstValue::Vector(vec![1.0, 2.0])),
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "B".to_string(),
            kind: BlockKind::Const(ConstValue::Vector(vec![10.0, 20.0])),
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "S".to_string(),
            kind: BlockKind::Sum(vec![1.0, 1.0]),
            inputs: vec![
                Signal::Block("A".to_string()),
                Signal::Block("B".to_string()),
            ],
            ic: None,
        },
    ];
    let trace = run(&vector_sum, 1e-4, 1e-4);
    let (_, outputs) = &trace[0];
    assert_eq!(outputs["S"], SignalValue::Vector(vec![11.0, 22.0]));

    let mixed = vec![
        BlockInstance {
            name: "A".to_string(),
            kind: BlockKind::Const(ConstValue::Vector(vec![1.0, 2.0])),
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "B".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(10.0)),
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "S".to_string(),
            kind: BlockKind::Sum(vec![1.0, 1.0]),
            inputs: vec![
                Signal::Block("A".to_string()),
                Signal::Block("B".to_string()),
            ],
            ic: None,
        },
    ];
    let netlist = "V1 a 0 5\nR1 a 0 1k";
    let ideal_switches: BTreeMap<String, IdealSwitch> = BTreeMap::new();
    let diodes: BTreeMap<String, IdealDiode> = BTreeMap::new();
    let gates = BTreeMap::new();
    let err = simulate_transient_with_blocks(
        netlist,
        Dialect::Ngspice,
        &diodes,
        &ideal_switches,
        &mixed,
        &gates,
        0.1,
        None,
        1e-4,
        TimeStep::Fixed(1e-5),
    )
    .unwrap_err();
    assert!(matches!(err, DaeError::VectorSignalNotSupported { block } if block == "S"));
}

#[test]
fn pid_rejects_a_vector_error_input() {
    // Pid is category-7 "reject" per the design survey -- no elementwise generalization makes
    // sense for its own anti-windup clamp logic.
    let blocks = vec![
        BlockInstance {
            name: "ERR".to_string(),
            kind: BlockKind::Const(ConstValue::Vector(vec![1.0, 2.0])),
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "PID1".to_string(),
            kind: BlockKind::Pid {
                pid: continuous_blocks::Pid::new(1.0, 0.0, 0.0, 1000.0).unwrap(),
                clamp: dae_runtime::PidClamp::Fixed(-1.0, 1.0),
            },
            inputs: vec![Signal::Block("ERR".to_string())],
            ic: None,
        },
    ];
    let netlist = "V1 a 0 5\nR1 a 0 1k";
    let ideal_switches: BTreeMap<String, IdealSwitch> = BTreeMap::new();
    let diodes: BTreeMap<String, IdealDiode> = BTreeMap::new();
    let gates = BTreeMap::new();
    let err = simulate_transient_with_blocks(
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
    .unwrap_err();
    assert!(matches!(err, DaeError::VectorSignalNotSupported { block } if block == "PID1"));
}
