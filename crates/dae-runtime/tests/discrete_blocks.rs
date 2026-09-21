//! Discrete state-space/transfer-function/PID through the real block-graph evaluation loop --
//! confirms the mandatory `sample_time` zero-order-hold gating (a genuinely new parsing/runtime
//! rule shape -- see `book/dev-guide/src/discrete-time-blocks.md`) actually holds `dt`-driven
//! circuit steps between sample instants, and that `discrete_step`/`DiscretePid::step` are only
//! invoked when due, not just that the underlying math is right (already covered by
//! `continuous-blocks`'s own unit tests).

use std::collections::BTreeMap;

use continuous_blocks::{DiscreteIntegrationMethod, DiscretePid, StateSpace, TransferFunction};
use dae_runtime::{
    simulate_transient_with_blocks, BlockInstance, BlockKind, ConstValue, PidClamp, SampleTimeSpec,
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

fn value_at(trace: &[(f64, BTreeMap<String, SignalValue>)], t_target: f64, name: &str) -> f64 {
    trace
        .iter()
        .find(|(t, _)| (*t - t_target).abs() < 1e-9)
        .unwrap_or_else(|| panic!("no row at t={t_target}"))
        .1[name]
        .as_scalar()
        .unwrap()
}

#[test]
fn discretestatespace_only_advances_on_its_own_sample_period_holding_between_hits() {
    // x[i+1] = 0.5*x[i] + 2*u[i], u held at 1.0 throughout, ts=0.1, circuit dt=0.01. This
    // block's own `time_since_sample` starts at `period - offset` = 0.1 (see BlockState's own
    // init, matching kind=cscript's pre-existing "always due on the first evaluate_blocks call"
    // convention) -- so the *first* hit lands on the first circuit step (t=0.01: x=0.5*0+2*1=2),
    // then every 0.1s after that (t=0.11: x=0.5*2+2=3; t=0.21: x=0.5*3+2=3.5), held flat between
    // hits.
    let blocks = vec![
        BlockInstance {
            name: "U".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(1.0)),
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "SS".to_string(),
            kind: BlockKind::DiscreteStateSpace {
                ss: StateSpace::new(
                    vec![vec![0.5]],
                    vec![vec![2.0]],
                    vec![vec![1.0]],
                    vec![vec![0.0]],
                    None,
                )
                .unwrap(),
                sample_time: SampleTimeSpec::Periodic {
                    period: 0.1,
                    offset: 0.0,
                },
            },
            inputs: vec![Signal::Block("U".to_string())],
            ic: None,
        },
    ];
    let trace = run(&blocks, 0.25, 0.01);
    assert!((value_at(&trace, 0.01, "SS") - 2.0).abs() < 1e-9); // first hit: 0.5*0+2*1=2
    assert!((value_at(&trace, 0.05, "SS") - 2.0).abs() < 1e-9); // held, not yet due again
    assert!((value_at(&trace, 0.10, "SS") - 2.0).abs() < 1e-9); // still held
    assert!((value_at(&trace, 0.11, "SS") - 3.0).abs() < 1e-9); // due: 0.5*2+2*1=3
    assert!((value_at(&trace, 0.20, "SS") - 3.0).abs() < 1e-9); // held again
    assert!((value_at(&trace, 0.21, "SS") - 3.5).abs() < 1e-9); // due: 0.5*3+2*1=3.5
}

#[test]
fn discretetf_matches_the_equivalent_discretestatespace_realization() {
    // Y(z)/U(z) = 2/(z-0.5) -- the z-domain transfer function whose controllable-canonical-form
    // realization is exactly the state-space system used above, so both must agree step for step.
    let blocks = vec![
        BlockInstance {
            name: "U".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(1.0)),
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "TF".to_string(),
            kind: BlockKind::DiscreteTransferFunction {
                tf: TransferFunction::new(vec![2.0], vec![1.0, -0.5]).unwrap(),
                sample_time: SampleTimeSpec::Periodic {
                    period: 0.1,
                    offset: 0.0,
                },
            },
            inputs: vec![Signal::Block("U".to_string())],
            ic: None,
        },
    ];
    let trace = run(&blocks, 0.25, 0.01);
    assert!((value_at(&trace, 0.01, "TF") - 2.0).abs() < 1e-9);
    assert!((value_at(&trace, 0.10, "TF") - 2.0).abs() < 1e-9);
    assert!((value_at(&trace, 0.11, "TF") - 3.0).abs() < 1e-9);
    assert!((value_at(&trace, 0.21, "TF") - 3.5).abs() < 1e-9);
}

#[test]
fn discretepid_only_advances_on_its_own_sample_period() {
    // Pure integral action (Forward Euler), constant error=2.0, Ki=5, ts=0.1 -- same hand-derived
    // sequence as continuous_blocks::discrete_pid's own unit test, now checked through the real
    // evaluation loop with a much finer circuit dt underneath it.
    let blocks = vec![
        BlockInstance {
            name: "ERR".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(2.0)),
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "CTRL".to_string(),
            kind: BlockKind::DiscretePid {
                pid: DiscretePid {
                    kp: 0.0,
                    ki: 5.0,
                    kd: 0.0,
                    n: 1.0,
                    period: 0.1,
                    method: DiscreteIntegrationMethod::ForwardEuler,
                },
                clamp: PidClamp::Fixed(-100.0, 100.0),
                sample_time: SampleTimeSpec::Periodic {
                    period: 0.1,
                    offset: 0.0,
                },
            },
            inputs: vec![Signal::Block("ERR".to_string())],
            ic: None,
        },
    ];
    let trace = run(&blocks, 0.35, 0.01);
    assert_eq!(value_at(&trace, 0.05, "CTRL"), 0.0); // held before the first hit
    assert!((value_at(&trace, 0.10, "CTRL") - 0.0).abs() < 1e-9); // first hit: no feedthrough
    assert!((value_at(&trace, 0.20, "CTRL") - 1.0).abs() < 1e-9);
    assert!((value_at(&trace, 0.30, "CTRL") - 2.0).abs() < 1e-9);
}

#[test]
fn discretepid_anti_windup_rejects_a_step_that_would_saturate_further() {
    // Ki large enough that the very first hit would blow straight past a tight clamp -- the
    // committed state must not advance past what the clamp allows, exactly like the continuous
    // Pid's own tentative-step/reject arm.
    let blocks = vec![
        BlockInstance {
            name: "ERR".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(2.0)),
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "CTRL".to_string(),
            kind: BlockKind::DiscretePid {
                pid: DiscretePid {
                    kp: 0.0,
                    ki: 5.0,
                    kd: 0.0,
                    n: 1.0,
                    period: 0.1,
                    method: DiscreteIntegrationMethod::BackwardEuler, // feedthrough: reacts on hit 1
                },
                clamp: PidClamp::Fixed(-0.5, 0.5),
                sample_time: SampleTimeSpec::Periodic {
                    period: 0.1,
                    offset: 0.0,
                },
            },
            inputs: vec![Signal::Block("ERR".to_string())],
            ic: None,
        },
    ];
    let trace = run(&blocks, 0.25, 0.01);
    // Unclamped, BackwardEuler's first hit would be 5*0.1*2=1.0 -- clamp holds it to 0.5.
    assert!((value_at(&trace, 0.10, "CTRL") - 0.5).abs() < 1e-9);
    // Error is still positive at the next hit, output already at hi=0.5 -- anti-windup must
    // reject the further step, holding output at exactly 0.5 rather than integrating past it.
    assert!((value_at(&trace, 0.20, "CTRL") - 0.5).abs() < 1e-9);
}
