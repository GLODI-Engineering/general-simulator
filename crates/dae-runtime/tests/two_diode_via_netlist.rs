//! The same circuit and hand-derived expected values as
//! `pwl-devices/tests/two_diode_circuit.rs`'s `two_pwl_diodes_reproduce_hand_derived_operating_point`,
//! but this time built from real netlist text through `general-mna`'s `'D'` stamp and
//! `dae-runtime`'s *generic* Thevenin/LCP fold, instead of a hand-typed `(M, q)` for this one
//! topology. Reproducing the same, already-independently-verified numbers via a structurally
//! different (general-purpose) code path is real evidence the generic fold is correct — not
//! just that the code runs.

use std::collections::BTreeMap;

use dae_runtime::solve_dc;
use general_spice_core::Dialect;
use pwl_devices::Diode;

#[test]
fn generic_fold_reproduces_the_hand_derived_two_diode_operating_point() {
    let netlist = "V1 e1 0 5\nD1 e1 e2 dmodel\nD2 e1 e2 dmodel\nR1 e2 0 1";

    let mut diodes = BTreeMap::new();
    diodes.insert("D1".to_string(), Diode::new(0.0, -100.0, 0.0, 1.0, 1.0));
    diodes.insert("D2".to_string(), Diode::new(0.0, -100.0, 0.0, 2.0, 1.0));

    let op = solve_dc(netlist, Dialect::Ngspice, &diodes).expect("circuit must solve");

    let e1 = op.value("V(e1)").expect("e1 must be an unknown");
    let e2 = op.value("V(e2)").expect("e2 must be an unknown");
    let v = e1 - e2;

    assert!(
        (e1 - 5.0).abs() < 1e-9,
        "e1 = {e1}, expected 5 (fixed by V1)"
    );
    assert!((e2 - 7.0 / 3.0).abs() < 1e-6, "e2 = {e2}, expected 7/3");
    assert!((v - 8.0 / 3.0).abs() < 1e-6, "V = {v}, expected 8/3");

    let d1 = Diode::new(0.0, -100.0, 0.0, 1.0, 1.0);
    let d2 = Diode::new(0.0, -100.0, 0.0, 2.0, 1.0);
    assert!(
        (d1.current(v) - 5.0 / 3.0).abs() < 1e-6,
        "I_D1 expected 5/3"
    );
    assert!(
        (d2.current(v) - 2.0 / 3.0).abs() < 1e-6,
        "I_D2 expected 2/3"
    );

    // Both diodes should have resolved into their forward-conduction segment: z1 == 0 (not in
    // breakdown), z2 > 0 (above threshold).
    for (name, (z1, z2)) in op.diode_names.iter().zip(op.diode_z.iter()) {
        assert!(
            z1.abs() < 1e-9,
            "{name} should not be in breakdown: z1={z1}"
        );
        assert!(*z2 > 0.0, "{name} should be forward-conducting: z2={z2}");
    }
}

/// The lower-source-voltage case from the same `pwl-devices` fixture: only D1 conducts.
#[test]
fn generic_fold_reproduces_only_first_diode_conducting() {
    let netlist = "V1 e1 0 1.5\nD1 e1 e2 dmodel\nD2 e1 e2 dmodel\nR1 e2 0 1";

    let mut diodes = BTreeMap::new();
    diodes.insert("D1".to_string(), Diode::new(0.0, -100.0, 0.0, 1.0, 1.0));
    diodes.insert("D2".to_string(), Diode::new(0.0, -100.0, 0.0, 2.0, 1.0));

    let op = solve_dc(netlist, Dialect::Ngspice, &diodes).expect("circuit must solve");

    let e1 = op.value("V(e1)").unwrap();
    let e2 = op.value("V(e2)").unwrap();
    let v = e1 - e2;

    assert!((v - 1.25).abs() < 1e-6, "V = {v}, expected 1.25");
    assert!((e2 - 0.25).abs() < 1e-6, "e2 = {e2}, expected 0.25");

    let (_, d1_z) = (&op.diode_names[0], op.diode_z[0]);
    let (_, d2_z) = (&op.diode_names[1], op.diode_z[1]);
    assert!(d1_z.1 > 0.0, "D1 should be forward-conducting");
    assert!(d2_z.1.abs() < 1e-9, "D2 should be off");
}
