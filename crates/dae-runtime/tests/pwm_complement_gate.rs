//! `BlockKind::Pwm` (PWM Modulator 1) — fixed-frequency, duty-driven, active-high complementary
//! PWM (fused from the old `GateBinding::Pwm`/`PwmComplement` pair into one component with two
//! outputs). Checks
//! two independent MOSFETs, one gated off the `main` output and one off `complement`, both
//! wired through their own `sig2gate` converter: with zero dead time, at every sampled instant
//! exactly one of the two must be conducting (no shoot-through gap, no overlap); with nonzero
//! dead time, both must be off during each dead-time gap, verified against hand-computed
//! switching instants, the same way every other gate-timing behavior in this crate is checked.

use std::collections::BTreeMap;

use dae_runtime::{
    simulate_transient_with_blocks, BlockInstance, BlockKind, GateBinding, Signal, TimeStep,
};
use general_spice_core::Dialect;
use pwl_devices::{Diode, Mosfet};

fn setup() -> (
    &'static str,
    BTreeMap<String, Mosfet>,
    BTreeMap<String, Diode>,
) {
    let netlist =
        "V1 a 0 5\nD1 a b mosfetmodel\nR1 b 0 1000\nV2 c 0 5\nD2 c d mosfetmodel\nR2 d 0 1000";
    let mosfet_top = Mosfet::new(0.1, Diode::new(0.0, -1e6, 1e-6, 1e6, 0.0));
    let mosfet_bot = Mosfet::new(0.1, Diode::new(0.0, -1e6, 1e-6, 1e6, 0.0));
    let mut mosfets = BTreeMap::new();
    mosfets.insert("D1".to_string(), mosfet_top);
    mosfets.insert("D2".to_string(), mosfet_bot);
    (netlist, mosfets, BTreeMap::new())
}

fn pwm_blocks(freq_hz: f64, red: f64, fed: f64) -> Vec<BlockInstance> {
    vec![
        BlockInstance {
            name: "DUTY".to_string(),
            kind: BlockKind::Const(0.3),
            inputs: vec![],
        },
        BlockInstance {
            name: "MOD".to_string(),
            kind: BlockKind::Pwm {
                freq_hz,
                red,
                fed,
                output_names: vec!["MOD".to_string(), "MOD_COMP".to_string()],
            },
            inputs: vec![Signal::Block("DUTY".to_string())],
        },
        BlockInstance {
            name: "MOD_MAIN_GATE".to_string(),
            kind: BlockKind::Sig2Gate,
            // The primary output is always bound to the block's own name ("MOD"), not
            // output_names[0] -- see evaluate_blocks' own `outputs.insert(block.name...)`.
            inputs: vec![Signal::Block("MOD".to_string())],
        },
        BlockInstance {
            name: "MOD_COMP_GATE".to_string(),
            kind: BlockKind::Sig2Gate,
            inputs: vec![Signal::Block("MOD_COMP".to_string())],
        },
    ]
}

#[test]
fn pwm_complement_is_mutually_exclusive_with_pwm_at_zero_deadtime() {
    let (netlist, mosfets, diodes) = setup();
    let blocks = pwm_blocks(100_000.0, 0.0, 0.0);

    let mut gates = BTreeMap::new();
    gates.insert(
        "D1".to_string(),
        GateBinding::Block("MOD_MAIN_GATE".to_string()),
    );
    gates.insert(
        "D2".to_string(),
        GateBinding::Block("MOD_COMP_GATE".to_string()),
    );

    let trace = simulate_transient_with_blocks(
        netlist,
        Dialect::Ngspice,
        &diodes,
        &mosfets,
        &blocks,
        &gates,
        0.1,
        None,
        20e-6, // two full 10us periods
        TimeStep::Fixed(1e-8),
    )
    .unwrap();

    // D1 (main) on for carrier<0.3, i.e. the first 3us of each 10us period; D2 (complement) on
    // for the remaining 7us. On at V()~5 (R/(R+Ron)~1), off at V()~0 (leakage only).
    let mut both_on_count = 0;
    let mut neither_on_count = 0;
    for (t, point, _) in &trace {
        let carrier = (t / 10e-6).fract();
        let d1_on = carrier < 0.3;
        let d2_on = carrier >= 0.3;
        let vb = point.value("V(b)").unwrap();
        let vd = point.value("V(d)").unwrap();
        let d1_conducting = vb > 2.5;
        let d2_conducting = vd > 2.5;
        assert_eq!(
            d1_conducting, d1_on,
            "t={t}: D1 conduction mismatch (V(b)={vb})"
        );
        assert_eq!(
            d2_conducting, d2_on,
            "t={t}: D2 conduction mismatch (V(d)={vd})"
        );
        if d1_conducting && d2_conducting {
            both_on_count += 1;
        }
        if !d1_conducting && !d2_conducting {
            neither_on_count += 1;
        }
    }
    assert_eq!(both_on_count, 0, "no instant should have both switches on");
    assert_eq!(
        neither_on_count, 0,
        "no instant should have both switches off"
    );
}

#[test]
fn nonzero_deadtime_leaves_both_switches_off_during_each_gap() {
    // freq=100kHz (period=10us), duty=0.3 (main on for the first 3us), red=200ns, fed=300ns:
    // main on [0.2us, 3us), complement on [3.3us, 10us) -- two dead gaps per period,
    // [0,0.2us) and [3us,3.3us), where both must be off.
    let (netlist, mosfets, diodes) = setup();
    let blocks = pwm_blocks(100_000.0, 200e-9, 300e-9);

    let mut gates = BTreeMap::new();
    gates.insert(
        "D1".to_string(),
        GateBinding::Block("MOD_MAIN_GATE".to_string()),
    );
    gates.insert(
        "D2".to_string(),
        GateBinding::Block("MOD_COMP_GATE".to_string()),
    );

    let trace = simulate_transient_with_blocks(
        netlist,
        Dialect::Ngspice,
        &diodes,
        &mosfets,
        &blocks,
        &gates,
        0.1,
        None,
        10e-6,
        TimeStep::Fixed(1e-8),
    )
    .unwrap();

    let mut both_on_count = 0;
    for (t, point, _) in &trace {
        let carrier_us = (t % 10e-6) * 1e6;
        let vb = point.value("V(b)").unwrap();
        let vd = point.value("V(d)").unwrap();
        let d1_conducting = vb > 2.5;
        let d2_conducting = vd > 2.5;
        if d1_conducting && d2_conducting {
            both_on_count += 1;
        }
        if (0.05..0.15).contains(&carrier_us) || (3.05..3.15).contains(&carrier_us) {
            assert!(
                !d1_conducting && !d2_conducting,
                "t={t} (carrier={carrier_us}us): expected both off during dead-time gap"
            );
        }
    }
    assert_eq!(both_on_count, 0, "no instant should have both switches on");
}
