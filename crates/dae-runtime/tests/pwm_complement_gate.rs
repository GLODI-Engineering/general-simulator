//! `GateBinding::PwmComplement` — the exact logical complement of `GateBinding::Pwm`, meant to
//! drive a half-bridge leg's two switches from one shared duty command. Checks two independent
//! MOSFETs, one `Pwm` and one `PwmComplement`, both naming the same duty block and frequency:
//! at every sampled instant, exactly one of the two must be conducting (no shoot-through gap,
//! no overlap) — verified against hand-computed switching instants, the same way every other
//! gate-timing behavior in this crate is checked.

use std::collections::BTreeMap;

use dae_runtime::{
    simulate_transient_with_blocks, BlockInstance, BlockKind, GateBinding, Signal, TimeStep,
};
use pwl_devices::{Diode, Mosfet};
use spice_core::Dialect;

#[test]
fn pwm_complement_is_mutually_exclusive_with_pwm_at_shared_duty_and_freq() {
    // Two independent branches so both switches' conduction is separately observable: D1 (top)
    // a -> b -> ground through R1, D2 (bottom) c -> d -> ground through R2, both driven off the
    // same duty=0.3, freq=100kHz carrier.
    let netlist =
        "V1 a 0 5\nD1 a b mosfetmodel\nR1 b 0 1000\nV2 c 0 5\nD2 c d mosfetmodel\nR2 d 0 1000";
    let mosfet_top = Mosfet::new(0.1, Diode::new(0.0, -1e6, 1e-6, 1e6, 0.0));
    let mosfet_bot = Mosfet::new(0.1, Diode::new(0.0, -1e6, 1e-6, 1e6, 0.0));
    let mut mosfets = BTreeMap::new();
    mosfets.insert("D1".to_string(), mosfet_top);
    mosfets.insert("D2".to_string(), mosfet_bot);
    let diodes = BTreeMap::new();

    let blocks = vec![
        BlockInstance {
            name: "DUTY".to_string(),
            kind: BlockKind::Const(0.3),
            inputs: vec![],
        },
        BlockInstance {
            name: "DUTY_GATE".to_string(),
            kind: BlockKind::Sig2Gate,
            inputs: vec![Signal::Block("DUTY".to_string())],
        },
    ];

    let mut gates = BTreeMap::new();
    gates.insert(
        "D1".to_string(),
        GateBinding::Pwm {
            duty: "DUTY_GATE".to_string(),
            freq_hz: 100_000.0,
        },
    );
    gates.insert(
        "D2".to_string(),
        GateBinding::PwmComplement {
            duty: "DUTY_GATE".to_string(),
            freq_hz: 100_000.0,
        },
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

    // D1 (Pwm) on for carrier<0.3, i.e. the first 3us of each 10us period; D2 (PwmComplement)
    // on for the remaining 7us. On at V()~5 (R/(R+Ron)~1), off at V()~0 (leakage only).
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
