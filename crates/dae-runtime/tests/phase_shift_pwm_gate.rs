//! `BlockKind::PhaseShiftPwm` (PWM Modulator 2: the fusion of the old `GateBinding::Vco`/
//! `VcoPhase` plus a block-driven duty neither had) — checked against hand-computed switching
//! instants, the way every other gate-timing behavior in this crate is verified, not just "it
//! ran."
//!
//! Circuit: `V1 (10V) -- D1 (ideal switch, drain=a, source=b, plain SPICE node order) -- R1 (1k) --
//! ground`. Chosen deliberately (not just "the natural declaration") so the body diode's real
//! forward direction (anode=source=b, cathode=drain=a, i.e. `b -> a`) does *not* match the
//! normal charging direction (`a -> b`) — otherwise the body diode would keep conducting even
//! with the gate off, masking the OFF state (same bug already documented in this crate's own
//! `closed_loop_pi_regulator.rs` test).
//! `FREQ` fixed at `100kHz` (a `Const`, clamped internally to `[f_min, f_max]=[100000,
//! 100000]`, so the internal oscillator ramps linearly), `PHASE` a `Const(0.25)` block,
//! `DUTY` a `Const(0.5)` block, zero dead time. `main` on while `(ramp + phase).rem_euclid(1.0)
//! < duty`, i.e. `ramp` in `[-0.25, 0.25) mod 1 = [0, 0.25) ∪ [0.75, 1.0)`. At `100kHz` (`10µs`
//! period): ON for `t` in `[0, 2.5µs)`, OFF `[2.5, 7.5µs)`, ON `[7.5, 10µs)`, repeating. Sampled
//! at `t=1µs` (expect ON, `V(b) ≈ 10*R1/(R1+Ron) ≈ 9.999V`), `t=5µs` (expect OFF, `V(b) ≈ 0V`,
//! only the ideal switch's own tiny leakage conductance), `t=9µs` (expect ON again).

use std::collections::BTreeMap;

use dae_runtime::{
    simulate_transient_with_blocks, BlockInstance, BlockKind, ConstValue, GateBinding,
    PhysicalDomain, Signal, TimeStep,
};
use general_spice_core::Dialect;
use pwl_devices::{IdealDiode, IdealSwitch};

#[test]
fn phase_shift_pwm_gate_matches_hand_computed_switching_instants() {
    let netlist = "V1 a 0 10\nD1 a b idealswitchmodel\nR1 b 0 1k";
    let switch = IdealSwitch::new(0.1, IdealDiode::new(0.0, -100.0, 0.0, 0.7, 1.0));
    let mut ideal_switches = BTreeMap::new();
    ideal_switches.insert("D1".to_string(), switch);
    let diodes = BTreeMap::new();

    let blocks = vec![
        BlockInstance {
            name: "FREQ".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(100_000.0)),
            inputs: vec![],
        },
        BlockInstance {
            name: "PHASE".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(0.25)),
            inputs: vec![],
        },
        BlockInstance {
            name: "DUTY".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(0.5)),
            inputs: vec![],
        },
        BlockInstance {
            name: "MOD".to_string(),
            kind: BlockKind::PhaseShiftPwm {
                osc: continuous_blocks::Vco::new(100_000.0, 100_000.0).unwrap(),
                red: 0.0,
                fed: 0.0,
                output_names: vec!["MOD".to_string(), "MOD_COMP".to_string()],
            },
            inputs: vec![
                Signal::Block("FREQ".to_string()),
                Signal::Block("PHASE".to_string()),
                Signal::Block("DUTY".to_string()),
            ],
        },
        BlockInstance {
            name: "MOD_MAIN_GATE".to_string(),
            kind: BlockKind::Sig2Phys {
                domain: PhysicalDomain::Voltage,
            },
            // The primary output is always bound to the block's own name ("MOD"), not
            // output_names[0] -- see evaluate_blocks' own `outputs.insert(block.name...)`.
            inputs: vec![Signal::Block("MOD".to_string())],
        },
    ];

    let mut gates = BTreeMap::new();
    gates.insert(
        "D1".to_string(),
        GateBinding::Block("MOD_MAIN_GATE".to_string()),
    );

    let trace = simulate_transient_with_blocks(
        netlist,
        Dialect::Ngspice,
        &diodes,
        &ideal_switches,
        &blocks,
        &gates,
        0.1,
        None,
        10e-6,
        TimeStep::Fixed(1e-8),
    )
    .unwrap();

    let v_at = |target_t: f64| -> f64 {
        let (_, point, _) = trace
            .iter()
            .min_by(|(t1, ..), (t2, ..)| (t1 - target_t).abs().total_cmp(&(t2 - target_t).abs()))
            .unwrap();
        point.value("V(b)").unwrap()
    };

    let on_1us = v_at(1e-6);
    let off_5us = v_at(5e-6);
    let on_9us = v_at(9e-6);

    assert!(
        on_1us > 9.9,
        "expected ON (V(b)~9.999) at t=1us, got {on_1us}"
    );
    assert!(
        off_5us.abs() < 0.1,
        "expected OFF (V(b)~0) at t=5us, got {off_5us}"
    );
    assert!(
        on_9us > 9.9,
        "expected ON (V(b)~9.999) at t=9us, got {on_9us}"
    );
}
