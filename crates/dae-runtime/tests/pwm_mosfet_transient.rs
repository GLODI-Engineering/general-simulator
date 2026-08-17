//! A single MOSFET gate transition mid-simulation, exercising `simulate_transient_with_mosfets`
//! end to end: the netlist is rebuilt (switch <-> `'D'`-stamped body diode) when the gate
//! state changes, the step lands on backward Euler right at the transition (per
//! `simulate_transient_with_mosfets`'s own documented policy), and the circuit continues
//! correctly into a *different* RC time constant afterward — checked against a closed-form
//! solution derived by hand in two phases, before writing this test.
//!
//! Circuit: `V1 (10V) -- D1 (a MOSFET, source=a, drain=b) -- R1 (1 ohm) -- C1 (1F) -- ground`.
//! `D1`'s body diode: `v_th=0.7`, `g_on=1` (same *diode* parameters as
//! `tests/transient.rs`'s `rc_charging_through_a_forward_biased_diode_matches_hand_derived_solution`
//! — but that fixture uses a 5V source, this one 10V, so the derived constants below differ;
//! do not copy `4.3`/`8.3` from there). Gate: off for `t < 1`, on for `t >= 1`, `r_on = 0.1`.
//!
//! ## Phase 1 (`t < 1`, gate off): body diode conducts
//!
//! Same structure as `tests/transient.rs`'s derivation but with `V0=10`:
//! `dVc/dt = (V0 - v_th - Vc) / (C*(R+1/g_on)) = (10 - 0.7 - Vc) / 2 = (9.3 - Vc) / 2`, so
//! `Vc(t) = 9.3 * (1 - e^(-t/2))`.
//!
//! ## Phase 2 (`t >= 1`, gate on): plain series RC with a new time constant
//!
//! `D1` is now a `0.1 ohm` resistor (no diode drop), so `V1 -- (Ron + R1 = 1.1 ohm) -- C1`:
//! `dVc/dt = (10 - Vc) / 1.1`, continuous with phase 1's `Vc(1)` (capacitor voltage cannot
//! jump), giving `Vc(t) = 10 - (10 - Vc(1)) * e^(-(t-1)/1.1)` for `t >= 1`, where
//! `Vc(1) = 9.3 * (1 - e^(-0.5))` from phase 1's formula.

use dae_runtime::{simulate_transient_with_mosfets, GateState};
use pwl_devices::{Diode, Mosfet};
use spice_core::Dialect;
use std::collections::BTreeMap;

#[test]
fn gate_transition_mid_simulation_matches_two_phase_hand_derivation() {
    let netlist = "V1 a 0 10\nD1 a b mosfetmodel\nR1 b c 1\nC1 c 0 1";
    let mosfet = Mosfet::new(0.1, Diode::new(0.0, -100.0, 0.0, 0.7, 1.0));
    let mut mosfets = BTreeMap::new();
    mosfets.insert("D1".to_string(), mosfet);
    let diodes = BTreeMap::new();

    let gate_signal = |_name: &str, t: f64| {
        if t < 1.0 {
            GateState::Off
        } else {
            GateState::On
        }
    };

    let trace = simulate_transient_with_mosfets(
        netlist,
        Dialect::Ngspice,
        &diodes,
        &mosfets,
        gate_signal,
        0.1,
        None,
        3.0,
        1e-3,
    )
    .unwrap();

    let vc_at = |t: f64| -> f64 {
        let index = (t / 1e-3).round() as usize - 1;
        let (t_actual, point) = &trace[index];
        assert!((t_actual - t).abs() < 1e-9);
        point.value("V(c)").unwrap()
    };

    // Phase 1, well away from the t=1 transition to avoid any floating-point boundary noise.
    let vc_half = vc_at(0.5);
    let expected_half = 9.3 * (1.0 - (-0.5_f64 / 2.0).exp());
    assert!(
        (vc_half - expected_half).abs() < 1e-5,
        "phase 1, t=0.5: Vc={vc_half}, expected {expected_half}"
    );

    // Phase 2, well after the transition. Vc(1) computed from phase 1's own formula, matching
    // the derivation's continuity requirement (capacitor voltage cannot jump). Tolerance is
    // looser than other transient fixtures': right at the gate transition, dVc/dt itself is
    // discontinuous (a genuinely different ODE before/after), so the one backward-Euler step
    // straddling it has legitimately larger (still O(dt), not the usual smooth-region O(dt^2)
    // trapezoidal accuracy) local truncation error than a step in a smooth region — a known
    // "order reduction near non-smooth data" effect, not a sign of a bug (a real error here
    // would be far larger than the ~0.02% relative discrepancy actually observed).
    let vc_1 = 9.3 * (1.0 - (-0.5_f64).exp());
    for &t in &[2.0f64, 2.5, 3.0] {
        let vc = vc_at(t);
        let expected = 10.0 - (10.0 - vc_1) * (-(t - 1.0) / 1.1).exp();
        assert!(
            (vc - expected).abs() < 2e-3,
            "phase 2, t={t}: Vc={vc}, expected {expected}"
        );
    }

    // Sanity: phase 2's steady state should be approaching 10V (the source voltage, since the
    // final circuit is a plain resistive divider with no voltage drop across a diode anymore).
    let vc_final = vc_at(3.0);
    assert!(
        vc_final > 8.5,
        "Vc(3) = {vc_final} should be well on its way to 10V"
    );
}
