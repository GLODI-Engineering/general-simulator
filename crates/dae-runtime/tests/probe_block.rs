//! `BlockKind::Probe` — the *only* way a circuit quantity enters the signal domain (the
//! enforced physical/signal-domain boundary; see that type's own doc comment). Checks both
//! `ProbeTarget::Voltage` (`V(node)`) and `ProbeTarget::Current` (`I(branch)`) against a simple
//! RC circuit with a known closed-form transient, cross-referencing the same hand-derived
//! `Vout = Vin*(1-exp(-t/RC))` result this crate's own `rc_charging_matches_closed_form_
//! exponential` test already establishes for the underlying circuit solve — this test is about
//! the probe's own read path, not a fresh derivation of the RC response.

use std::collections::BTreeMap;

use dae_runtime::{
    simulate_transient_with_blocks, BlockInstance, BlockKind, ProbeTarget, TimeStep,
};
use general_spice_core::Dialect;
use pwl_devices::{IdealDiode, IdealSwitch};

#[test]
fn probe_voltage_and_current_match_hand_derived_rc_charging_curve() {
    // R1=1k, C1=1uF -> tau=1ms. No ideal switch needed at all -- calling
    // simulate_transient_with_blocks directly (not through the CLI, which only invokes the
    // block graph when at least one ideal switch is declared) runs the block graph fine with an
    // empty ideal_switches map.
    let netlist = "V1 a 0 10\nR1 a vout 1000\nC1 vout 0 1e-6";
    let ideal_switches: BTreeMap<String, IdealSwitch> = BTreeMap::new();
    let diodes: BTreeMap<String, IdealDiode> = BTreeMap::new();
    let gates = BTreeMap::new();

    let blocks = vec![
        BlockInstance {
            name: "VOUT_PROBE".to_string(),
            kind: BlockKind::Probe(ProbeTarget::Voltage("vout".to_string())),
            inputs: vec![],
        },
        // general-mna only gives V/L/E/H elements their own branch-current MNA unknown (a
        // plain resistor's current is derivable but not separately stored) -- probe V1's own
        // current instead, which in this series R-C circuit equals the same charging current.
        BlockInstance {
            name: "I_V1_PROBE".to_string(),
            kind: BlockKind::Probe(ProbeTarget::Current("V1".to_string())),
            inputs: vec![],
        },
    ];

    let tau = 1000.0 * 1e-6; // R*C = 1ms
    let dt = tau / 500.0;
    let t_final = tau * 8.0; // exp(-8) ~= 3e-4, comfortably decayed

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

    // A probe reads the circuit's *previous* step (the same sampled-data convention every
    // block-graph read of circuit state uses) -- step i's own probe output must equal step
    // (i-1)'s resolved V(vout) exactly (step 0's probe reads the pre-run all-zero initial
    // point, i.e. 0.0), and only *then* is that value close to the analytic curve evaluated one
    // dt earlier than the step's own timestamp.
    let mut max_v_err = 0.0_f64;
    let mut prev_vout = 0.0_f64;
    for (t, point, outputs) in &trace {
        let expected_vout_prev = 10.0 * (1.0 - (-(t - dt) / tau).exp());
        let probed_vout = outputs["VOUT_PROBE"].as_scalar().unwrap();
        assert!(
            (probed_vout - prev_vout).abs() < 1e-12,
            "t={t}: probe VOUT_PROBE={probed_vout} disagrees with the previous step's own \
             V(vout)={prev_vout}"
        );
        max_v_err = max_v_err.max((probed_vout - expected_vout_prev).abs());
        prev_vout = point.value("V(vout)").unwrap();
    }
    assert!(
        max_v_err < 0.05,
        "probed V(vout) deviates from the hand-derived RC charging curve (evaluated one dt \
         earlier, matching the probe's own previous-step read) by {max_v_err}"
    );

    // I(V1) probe: current through the source into a charging capacitor should start near
    // decay toward 0. Step 0's probe reads the pre-run all-zero initial point (current
    // genuinely 0 there, not yet meaningful); step 1 already reads a solved point with
    // substantial charging current, so that's the right index for the "not near zero yet"
    // check.
    let (_, _, second_outputs) = &trace[1];
    let (_, _, last_outputs) = trace.last().unwrap();
    assert!(
        second_outputs["I_V1_PROBE"].as_scalar().unwrap().abs() > 0.005,
        "expected substantial charging current shortly after t=0, got {}",
        second_outputs["I_V1_PROBE"].as_scalar().unwrap()
    );
    assert!(
        last_outputs["I_V1_PROBE"].as_scalar().unwrap().abs() < 1e-4,
        "expected charging current to have decayed near zero by t_final, got {}",
        last_outputs["I_V1_PROBE"].as_scalar().unwrap()
    );
}
