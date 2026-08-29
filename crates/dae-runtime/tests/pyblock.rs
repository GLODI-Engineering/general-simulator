//! `kind=pyblock` through the real block-graph evaluation loop -- the Python-hosted counterpart
//! to `cscript.rs`'s own end-to-end tests, exercising `BlockKind::PyBlock` construction, the
//! `sample_time`-independent per-step call, vector-signal shape preservation (unlike `cscript`'s
//! flat C array, each declared `inputs=` entry keeps its own scalar/vector shape), and the `xc`
//! contract through the same solver-driven RK4 stepper every other dynamic block uses.

use std::collections::BTreeMap;
use std::path::PathBuf;

use dae_runtime::{
    simulate_transient_with_blocks, BlockInstance, BlockKind, ConstValue, Signal, SignalValue,
    TimeStep,
};
use general_spice_core::Dialect;
use pwl_devices::{IdealDiode, IdealSwitch};

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
fn pyblock_pi_controller_matches_a_hand_computed_recursion() {
    // REF steps to 1.0 at t=0, error = REF - 0 (no feedback loop here, just checking the
    // pyblock's own recursion matches a plain Rust-side computation of the identical formula).
    let blocks = vec![
        BlockInstance {
            name: "ERR".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(1.0)),
            inputs: vec![],
        },
        BlockInstance {
            name: "PID".to_string(),
            kind: BlockKind::PyBlock {
                path: fixture("pi_controller.py"),
                output_names: vec!["PID".to_string()],
                sample_time: None,
                xc_count: 0,
            },
            inputs: vec![Signal::Block("ERR".to_string())],
        },
    ];

    let dt = 1e-4;
    let n_steps = 50;
    let trace = run(&blocks, dt * n_steps as f64, dt);
    assert_eq!(trace.len(), n_steps);

    const KP: f64 = 2.0;
    const KI: f64 = 50.0;
    let mut integral = 0.0_f64;
    for (i, (_, outputs)) in trace.iter().enumerate() {
        integral += 1.0 * dt;
        let expected = KP * 1.0 + KI * integral;
        let got = outputs["PID"].as_scalar().unwrap();
        assert!(
            (got - expected).abs() < 1e-9,
            "step {i}: PID={got}, expected={expected}"
        );
    }
}

#[test]
fn pyblock_inputs_keep_their_own_scalar_or_vector_shape() {
    let blocks = vec![
        BlockInstance {
            name: "VEC".to_string(),
            kind: BlockKind::Const(ConstValue::Vector(vec![1.0, 2.0, 3.0])),
            inputs: vec![],
        },
        BlockInstance {
            name: "SCAL".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(10.0)),
            inputs: vec![],
        },
        BlockInstance {
            name: "OUT".to_string(),
            kind: BlockKind::PyBlock {
                path: fixture("vector_shape.py"),
                output_names: vec!["OUT".to_string()],
                sample_time: None,
                xc_count: 0,
            },
            inputs: vec![
                Signal::Block("VEC".to_string()),
                Signal::Block("SCAL".to_string()),
            ],
        },
    ];
    let trace = run(&blocks, 1e-4, 1e-4);
    let (_, outputs) = &trace[0];
    // sum([1,2,3]) + 10 = 16, and the fixture's own assertions confirm the shapes weren't
    // silently flattened together (an AssertionError there would surface as a PyBlockError,
    // failing this test with a clear message instead of a wrong number).
    assert_eq!(outputs["OUT"], SignalValue::Scalar(16.0));
}

#[test]
fn pyblock_xc_matches_the_closed_form_step_response() {
    // dx/dt = -K*x + u (K=2.0), u=10 constant, x(0)=0 -> x(t) = (u/K)*(1-exp(-K*t)), the same
    // closed-form check used for cscript's own xc contract and the vector-signals experiment's
    // MIMO comparison.
    let blocks = vec![
        BlockInstance {
            name: "U".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(10.0)),
            inputs: vec![],
        },
        BlockInstance {
            name: "X".to_string(),
            kind: BlockKind::PyBlock {
                path: fixture("decay_xc.py"),
                output_names: vec!["X".to_string()],
                sample_time: None,
                xc_count: 1,
            },
            inputs: vec![Signal::Block("U".to_string())],
        },
    ];

    let k = 2.0_f64;
    let u = 10.0_f64;
    let dt = 1e-4;
    let n_steps = 100;
    let trace = run(&blocks, dt * n_steps as f64, dt);
    let mut max_err = 0.0_f64;
    for (t, outputs) in &trace {
        let expected = (u / k) * (1.0 - (-k * t).exp());
        let got = outputs["X"].as_scalar().unwrap();
        max_err = max_err.max((got - expected).abs());
    }
    assert!(
        max_err < 1e-6,
        "X deviates from the closed-form step response by {max_err}"
    );
}
