//! Backward-Euler transient verification, in increasing order of what's being exercised:
//! (1) a purely algebraic circuit (no storage) must reproduce the exact DC answer already
//! verified in `two_diode_via_netlist.rs`, at every step, immediately; (2) a purely linear RC
//! charging circuit must match its closed-form exponential; (3) an RC circuit charging
//! *through a forward-biased diode* combines both mechanisms and is checked against a
//! closed-form solution derived by hand before writing this test (see the module doc below).

use dae_runtime::{simulate_transient, solve_dc, TimeStep};
use general_spice_core::Dialect;
use pwl_devices::Diode;
use std::collections::BTreeMap;

/// A purely algebraic (no `L`/`C`) circuit has no dynamics at all: `K = 0` everywhere, so the
/// backward-Euler fold `(A + K/dt)x = u + (K/dt)x_prev` reduces to exactly `A x = u` regardless
/// of `dt` or `x_prev`. Reusing the exact two-diode circuit from `two_diode_via_netlist.rs`,
/// its transient trace should equal that already-hand-verified DC answer at every single step,
/// not just eventually.
#[test]
fn algebraic_circuit_matches_dc_solve_at_every_step() {
    let netlist = "V1 e1 0 5\nD1 e1 e2 dmodel\nD2 e1 e2 dmodel\nR1 e2 0 1";
    let mut diodes = BTreeMap::new();
    diodes.insert("D1".to_string(), Diode::new(0.0, -100.0, 0.0, 1.0, 1.0));
    diodes.insert("D2".to_string(), Diode::new(0.0, -100.0, 0.0, 2.0, 1.0));

    let dc = solve_dc(netlist, Dialect::Ngspice, &diodes).unwrap();
    let trace = simulate_transient(
        netlist,
        Dialect::Ngspice,
        &diodes,
        None,
        0.5,
        TimeStep::Fixed(0.1),
    )
    .unwrap();

    assert_eq!(trace.len(), 5);
    for (t, point) in &trace {
        let e2 = point.value("V(e2)").unwrap();
        let dc_e2 = dc.value("V(e2)").unwrap();
        assert!(
            (e2 - dc_e2).abs() < 1e-9,
            "at t={t}: e2={e2}, expected {dc_e2} (the DC value)"
        );
    }
}

/// V1 (5V step) -> R1 (1 ohm) -> C1 (1F) -> ground. Standard RC charging:
/// dVc/dt = (5 - Vc) / (R*C) = (5 - Vc) / 1, so Vc(t) = 5*(1 - e^(-t)).
/// `simulate_transient` uses trapezoidal (second-order) for essentially the whole run here (a
/// linear circuit never triggers the backward-Euler mode-change fallback beyond the mandatory
/// first step), so the tolerance is tight — see `src/lib.rs`'s `scheme_tests` module for the
/// dedicated convergence-order proof this relies on.
#[test]
fn rc_charging_matches_closed_form_exponential() {
    let netlist = "V1 a 0 5\nR1 a b 1\nC1 b 0 1";
    let diodes = BTreeMap::new();

    let trace = simulate_transient(
        netlist,
        Dialect::Ngspice,
        &diodes,
        None,
        3.0,
        TimeStep::Fixed(1e-3),
    )
    .unwrap();

    for &t in &[0.5f64, 1.0, 2.0, 3.0] {
        let index = (t / 1e-3).round() as usize - 1;
        let (t_actual, point) = &trace[index];
        assert!((t_actual - t).abs() < 1e-9);
        let vc = point.value("V(b)").unwrap();
        let expected = 5.0 * (1.0 - (-t).exp());
        assert!(
            (vc - expected).abs() < 1e-5,
            "at t={t}: Vc={vc}, expected {expected} (5*(1-e^-t))"
        );
    }
}

/// V1 (5V step) -> D1 (v_th=0.7, g_on=1, forward: anode at a) -> R1 (1 ohm) -> C1 (1F) ->
/// ground. Single series loop, so one current I(t) = C*dVc/dt flows through everything.
///
/// KVL: 5 = V_diode_drop + I*R + Vc, and V_diode_drop = v_th + I/g_on (from the diode's
/// forward segment, I = g_on*(V_diode - v_th)).
/// => 5 = v_th + I*(1/g_on + R) + Vc = 0.7 + I*(1+1) + Vc = 0.7 + 2*I + Vc
/// => I = (4.3 - Vc) / 2, and I = C*dVc/dt = dVc/dt (C=1)
/// => dVc/dt = (4.3 - Vc) / 2, Vc(0)=0  =>  Vc(t) = 4.3*(1 - e^(-t/2))
///
/// This requires the diode to stay forward-conducting for the whole trajectory: current
/// I = dVc/dt = (4.3-Vc)/2 stays >= 0 throughout since Vc rises monotonically toward 4.3 from
/// below, so this self-consistency check holds for the entire simulated interval.
#[test]
fn rc_charging_through_a_forward_biased_diode_matches_hand_derived_solution() {
    let netlist = "V1 a 0 5\nD1 a b dmodel\nR1 b c 1\nC1 c 0 1";
    let mut diodes = BTreeMap::new();
    diodes.insert("D1".to_string(), Diode::new(0.0, -100.0, 0.0, 0.7, 1.0));

    let trace = simulate_transient(
        netlist,
        Dialect::Ngspice,
        &diodes,
        None,
        4.0,
        TimeStep::Fixed(1e-3),
    )
    .unwrap();

    for &t in &[0.5f64, 1.0, 2.0, 4.0] {
        let index = (t / 1e-3).round() as usize - 1;
        let (t_actual, point) = &trace[index];
        assert!((t_actual - t).abs() < 1e-9);
        let vc = point.value("V(c)").unwrap();
        let expected = 4.3 * (1.0 - (-t / 2.0).exp());
        assert!(
            (vc - expected).abs() < 1e-5,
            "at t={t}: Vc={vc}, expected {expected} (4.3*(1-e^(-t/2)))"
        );

        // The diode should be resolved forward-conducting (not off, not in breakdown) at
        // every checked point.
        let (name, (z1, z2)) = (&point.diode_names[0], point.diode_z[0]);
        assert!(
            z1.abs() < 1e-9,
            "{name} should not be in breakdown at t={t}: z1={z1}"
        );
        assert!(
            z2 > 0.0,
            "{name} should be forward-conducting at t={t}: z2={z2}"
        );
    }
}
