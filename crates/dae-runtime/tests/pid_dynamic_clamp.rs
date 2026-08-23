//! `PidClamp::Dynamic` — a `Pid` block's anti-windup bound read fresh from the graph every step
//! instead of fixed at model-build time. Added because a *fixed* bound sized for a controller's
//! final steady-state range can be badly oversized during a transient where the true achievable
//! range is still much smaller (e.g. a current-loop PID commanding a pole voltage bounded by
//! roughly half a DC bus voltage that's itself still ramping up) — the PID's own anti-windup
//! never engages even though the real plant is already saturated far below the fixed bound.
//!
//! Two checks: (1) with a *constant* dynamic bound, behavior matches `PidClamp::Fixed` at the
//! same numeric bound (parity with the existing, already-verified mechanism, just reached
//! through the new input-driven path); (2) with a bound that *shrinks* partway through the run,
//! the PID's output respects the new, tighter bound immediately after the step — the actual new
//! behavior only `Dynamic` enables, and the direct regression test for the failure mode that
//! motivated adding it (a `Fixed` bound could never do this at all).

use std::collections::BTreeMap;

use dae_runtime::{simulate_transient_with_blocks, BlockInstance, BlockKind, PidClamp, TimeStep};
use pwl_devices::{Diode, Mosfet};
use spice_core::Dialect;

const NETLIST: &str = "V1 a 0 5\nD1 a b mosfetmodel\nR1 b 0 1000";

fn dummy_mosfets() -> BTreeMap<String, Mosfet> {
    let mut m = BTreeMap::new();
    m.insert(
        "D1".to_string(),
        Mosfet::new(0.1, Diode::new(0.0, -100.0, 0.0, 1e6, 0.0)),
    );
    m
}

#[test]
fn dynamic_clamp_matches_fixed_clamp_at_the_same_constant_bound() {
    let mosfets = dummy_mosfets();
    let diodes = BTreeMap::new();
    let gates = BTreeMap::new();

    // A large constant error (kp=1, ki=1000) would drive the PID's unclamped output far past
    // +-5 within a handful of steps; both forms must hold it at exactly 5.0.
    let make_blocks = |clamp: PidClamp, inputs: Vec<dae_runtime::Signal>| {
        vec![
            BlockInstance {
                name: "ERR".to_string(),
                kind: BlockKind::Const(100.0),
                inputs: vec![],
            },
            BlockInstance {
                name: "LO".to_string(),
                kind: BlockKind::Const(-5.0),
                inputs: vec![],
            },
            BlockInstance {
                name: "HI".to_string(),
                kind: BlockKind::Const(5.0),
                inputs: vec![],
            },
            BlockInstance {
                name: "PID1".to_string(),
                kind: BlockKind::Pid {
                    pid: continuous_blocks::Pid::new(1.0, 1000.0, 0.0, 1000.0),
                    clamp,
                },
                inputs,
            },
        ]
    };

    let run = |blocks: &[BlockInstance]| {
        simulate_transient_with_blocks(
            NETLIST,
            Dialect::Ngspice,
            &diodes,
            &mosfets,
            blocks,
            &gates,
            0.1,
            None,
            2e-3,
            TimeStep::Fixed(1e-5),
        )
        .unwrap()
    };

    let fixed_blocks = make_blocks(
        PidClamp::Fixed(-5.0, 5.0),
        vec![dae_runtime::Signal::Block("ERR".to_string())],
    );
    let dynamic_blocks = make_blocks(
        PidClamp::Dynamic,
        vec![
            dae_runtime::Signal::Block("ERR".to_string()),
            dae_runtime::Signal::Block("LO".to_string()),
            dae_runtime::Signal::Block("HI".to_string()),
        ],
    );

    let fixed_trace = run(&fixed_blocks);
    let dynamic_trace = run(&dynamic_blocks);

    assert_eq!(fixed_trace.len(), dynamic_trace.len());
    for ((_, _, fixed_out), (_, _, dyn_out)) in fixed_trace.iter().zip(dynamic_trace.iter()) {
        assert_eq!(fixed_out["PID1"], dyn_out["PID1"]);
    }
    // Confirm the clamp is actually engaged (not just coincidentally equal), i.e. the run truly
    // saturated at the bound rather than staying comfortably inside it.
    assert_eq!(fixed_trace.last().unwrap().2["PID1"], 5.0);
}

#[test]
fn dynamic_clamp_respects_a_bound_that_shrinks_mid_run() {
    let mosfets = dummy_mosfets();
    let diodes = BTreeMap::new();
    let gates = BTreeMap::new();

    let blocks = vec![
        BlockInstance {
            name: "ERR".to_string(),
            kind: BlockKind::Const(100.0),
            inputs: vec![],
        },
        BlockInstance {
            name: "LO".to_string(),
            kind: BlockKind::Const(-1000.0),
            inputs: vec![],
        },
        // HI starts wide open (1000) so the PID's output is free to run up near it, then drops
        // to 5.0 at t=1ms -- the output must snap down to (or below) 5.0 immediately after,
        // which only Dynamic can do; a Fixed bound has no way to express this at all.
        BlockInstance {
            name: "HI".to_string(),
            kind: BlockKind::Pwc {
                points: vec![(0.0, 1000.0), (1e-3, 5.0)],
                repeat: false,
            },
            inputs: vec![],
        },
        BlockInstance {
            name: "PID1".to_string(),
            kind: BlockKind::Pid {
                pid: continuous_blocks::Pid::new(1.0, 1000.0, 0.0, 1000.0),
                clamp: PidClamp::Dynamic,
            },
            inputs: vec![
                dae_runtime::Signal::Block("ERR".to_string()),
                dae_runtime::Signal::Block("LO".to_string()),
                dae_runtime::Signal::Block("HI".to_string()),
            ],
        },
    ];

    let trace = simulate_transient_with_blocks(
        NETLIST,
        Dialect::Ngspice,
        &diodes,
        &mosfets,
        &blocks,
        &gates,
        0.1,
        None,
        2e-3,
        TimeStep::Fixed(1e-5),
    )
    .unwrap();

    // Just before the bound shrinks: output should have grown well past 5 (proving the wide
    // bound was genuinely in effect, not already saturated at the tighter one).
    let (_, _, before) = trace
        .iter()
        .rev()
        .find(|(t, ..)| *t < 1e-3)
        .expect("at least one step before t=1ms");
    assert!(
        before["PID1"] > 20.0,
        "expected PID1 to have grown well past 5 while HI=1000, got {}",
        before["PID1"]
    );

    // From the first step at/after the bound shrinks onward, output must never exceed 5.0.
    for (t, _, outputs) in trace.iter().filter(|(t, ..)| *t >= 1e-3) {
        assert!(
            outputs["PID1"] <= 5.0 + 1e-9,
            "t={t}: PID1={} exceeds the shrunk HI=5.0 bound",
            outputs["PID1"]
        );
    }
}
