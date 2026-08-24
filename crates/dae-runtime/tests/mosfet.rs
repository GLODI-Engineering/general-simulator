//! Hand-derived MOSFET fixtures. Per the model in `pwl_devices::Mosfet`'s doc comment: gated
//! on is a plain bidirectional `r_on` resistor (an existing, already-tested `general-mna`
//! mechanism); gated off falls back to the intrinsic body diode (the same LCP fold already
//! verified for ordinary diodes in `two_diode_via_netlist.rs`), stamped via
//! `Mosfet::body_diode_for_drain_source_stamping` so that a `Mosfet` element's two nodes are
//! declared plain SPICE-conventional `(drain, source)` — see that method's doc comment for why.

use std::collections::BTreeMap;

use dae_runtime::{solve_dc_with_mosfets, GateState};
use general_spice_core::Dialect;
use pwl_devices::{Diode, Mosfet};

/// Gated on: D1 is just a 0.1-ohm resistor from `a` to `b`, in series with a 10-ohm load to
/// ground, driven by a 5V source. I = 5 / (0.1 + 10) = 500/101 A. Vb = I * 10 = 5000/101 V.
#[test]
fn gated_on_mosfet_behaves_as_plain_on_resistance() {
    let netlist = "V1 a 0 5\nD1 a b mosfetmodel\nR1 b 0 10";
    let mosfet = Mosfet::new(0.1, Diode::new(0.0, -1e4, 0.0, 0.7, 1.0));
    let mut mosfets = BTreeMap::new();
    mosfets.insert("D1".to_string(), (mosfet, GateState::On));

    let op = solve_dc_with_mosfets(netlist, Dialect::Ngspice, &BTreeMap::new(), &mosfets, 0.1)
        .expect("circuit must solve");

    let expected_i = 5.0 / 10.1;
    let expected_vb = expected_i * 10.0;
    let va = op.value("V(a)").unwrap();
    let vb = op.value("V(b)").unwrap();
    assert!(
        (va - 5.0).abs() < 1e-9,
        "va = {va}, expected 5 (fixed by V1)"
    );
    assert!(
        (vb - expected_vb).abs() < 1e-6,
        "vb = {vb}, expected {expected_vb}"
    );
}

/// Gated off, body diode forward-conducting ("natural" commutation): declared (drain=a,
/// source=b) so V_body = Vb - Va. Vb fixed at 5V by an ideal source; Ra=1 from node a to
/// ground. Body diode: v_th=0.7, g_on=1.
///
/// KCL at a: I_diode(Vb - Va) = Va / Ra.
/// (5 - Va - 0.7) = Va  =>  4.3 = 2*Va  =>  Va = 2.15.
/// V_body = 5 - 2.15 = 2.85 > 0.7, confirming the forward-conducting guess is self-consistent.
#[test]
fn gated_off_mosfet_body_diode_conducts_when_pushed_backward() {
    let netlist = "Vb b 0 5\nD1 a b mosfetmodel\nRa a 0 1";
    let mosfet = Mosfet::new(0.1, Diode::new(0.0, -1e4, 0.0, 0.7, 1.0));
    let mut mosfets = BTreeMap::new();
    mosfets.insert("D1".to_string(), (mosfet, GateState::Off));

    let op = solve_dc_with_mosfets(netlist, Dialect::Ngspice, &BTreeMap::new(), &mosfets, 0.1)
        .expect("circuit must solve");

    let va = op.value("V(a)").unwrap();
    let vb = op.value("V(b)").unwrap();
    assert!((va - 2.15).abs() < 1e-6, "Va = {va}, expected 2.15");
    let v_body = vb - va;
    assert!(
        v_body > 0.7,
        "body diode should be forward-conducting: V_body = {v_body}"
    );
}

/// Gated off, blocking (no reverse push): Vb = 0, so V_body = -Va which stays below the 0.7 V
/// threshold as long as Va doesn't go negative — and with the diode off (I=0), KCL at `a`
/// forces Va = 0 exactly, which is indeed < 0.7, confirming the blocking guess.
#[test]
fn gated_off_mosfet_blocks_with_no_reverse_push() {
    let netlist = "Vb b 0 0\nD1 a b mosfetmodel\nRa a 0 1";
    let mosfet = Mosfet::new(0.1, Diode::new(0.0, -1e4, 0.0, 0.7, 1.0));
    let mut mosfets = BTreeMap::new();
    mosfets.insert("D1".to_string(), (mosfet, GateState::Off));

    let op = solve_dc_with_mosfets(netlist, Dialect::Ngspice, &BTreeMap::new(), &mosfets, 0.1)
        .expect("circuit must solve");

    let va = op.value("V(a)").unwrap();
    assert!(va.abs() < 1e-9, "Va = {va}, expected 0 (blocking)");
}
