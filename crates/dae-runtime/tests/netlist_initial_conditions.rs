//! A netlist's own `ic=` values, end to end through a real transient run.
//!
//! `general-mna` *assigns* them rather than solving a constrained operating point around them
//! (see `MnaSystem::initial_state`), so the seeded vector is deliberately not a consistent
//! operating point: the declared states carry their declared values and every other unknown
//! starts at zero. The run's very first step is backward Euler, which is what re-imposes the
//! algebraic constraints, and each expected number below is derived by hand from that first
//! step rather than read off a previous run.

use dae_runtime::{simulate_transient, DaeError, TimeStep};
use general_spice_core::Dialect;
use pwl_devices::IdealDiode;
use std::collections::BTreeMap;

fn no_diodes() -> BTreeMap<String, IdealDiode> {
    BTreeMap::new()
}

/// `V1 a 0 10` / `R1 a b 1k` / `C1 b 0 1u ic=5`, one fixed `dt = 10 us` step.
///
/// The assignment seeds `V(b) = 5` and leaves `V(a)` and `I(V1)` at zero. The first backward-
/// Euler step re-imposes `V(a) = 10` (V1's own branch equation) and solves node `b`'s KCL,
/// `(V(b) - V(a))/R + C*(V(b) - V(b0))/dt = 0`, i.e.
///
/// ```text
/// V(b) = (V(a)/R + C*V(b0)/dt) / (1/R + C/dt)
///      = (10/1000 + 1e-6*5/1e-5) / (1/1000 + 1e-6/1e-5)
///      = 0.51 / 0.101
///      = 5.049504950495050 V
/// ```
///
/// Starting from rest instead would give `0.01/0.101 = 0.0990099...`, two orders of magnitude
/// away, so this cannot pass by accident.
#[test]
fn a_netlist_ic_seeds_the_first_backward_euler_step() {
    let netlist = "V1 a 0 10\nR1 a b 1000\nC1 b 0 1e-6 ic=5";
    let trace = simulate_transient(
        netlist,
        Dialect::Ngspice,
        &no_diodes(),
        None,
        1e-5,
        TimeStep::Fixed(1e-5),
    )
    .unwrap();

    let (_, first) = &trace[0];
    let expected = 0.51 / 0.101;
    let vb = first.value("V(b)").unwrap();
    assert!(
        (vb - expected).abs() < 1e-12,
        "V(b) = {vb}, expected {expected}"
    );
    assert!((first.value("V(a)").unwrap() - 10.0).abs() < 1e-12);
}

/// `V1 a 0 10` / `L1 a b 5u ic=12` / `R1 b 0 10`, one fixed `dt = 10 ns` step.
///
/// The sign convention under test: `ic` is the current from the inductor's *first* node to its
/// *second*, so 12 A leave node `a`, pass through L1, and arrive at node `b`, where R1 carries
/// them to ground — `V(b)` must come out **positive**. The first backward-Euler step solves
/// `V(a) - V(b) = L*(i - i0)/dt` with `V(a) = 10` and `V(b) = 10*i`:
///
/// ```text
/// 10 - 10*i = (5e-6/1e-8)*(i - 12) = 500*i - 6000
/// 6010 = 510*i  =>  i = 11.784313725490196 A,  V(b) = 117.84313725490196 V
/// ```
#[test]
fn an_inductor_ic_enters_the_run_with_the_documented_sign() {
    let netlist = "V1 a 0 10\nL1 a b 5e-6 ic=12\nR1 b 0 10";
    let trace = simulate_transient(
        netlist,
        Dialect::Ngspice,
        &no_diodes(),
        None,
        1e-8,
        TimeStep::Fixed(1e-8),
    )
    .unwrap();

    let (_, first) = &trace[0];
    let expected = 6010.0 / 510.0;
    let current = first.value("I(L1)").unwrap();
    assert!(
        (current - expected).abs() < 1e-9,
        "I(L1) = {current}, expected {expected}"
    );
    assert!((first.value("V(b)").unwrap() - 10.0 * expected).abs() < 1e-7);
}

/// Reversing the card reverses the physical current while `I(L1)` keeps the same number, which
/// is exactly why the convention is written down: 12 A now leave node `b`, so R1 sees them
/// flowing the other way and `V(b)` comes out negative.
#[test]
fn reversing_the_inductor_card_reverses_the_physical_current() {
    let trace = simulate_transient(
        "V1 a 0 10\nL1 b a 5e-6 ic=12\nR1 b 0 10",
        Dialect::Ngspice,
        &no_diodes(),
        None,
        1e-8,
        TimeStep::Fixed(1e-8),
    )
    .unwrap();
    assert!(trace[0].1.value("V(b)").unwrap() < 0.0);
}

/// An explicit `x_initial` is the more specific instruction and still wins: handing over an
/// all-zero vector must reproduce the same deck with the `ic=` removed, step for step.
#[test]
fn an_explicit_x_initial_still_overrides_the_netlist_ic() {
    let with_ic = simulate_transient(
        "V1 a 0 10\nR1 a b 1000\nC1 b 0 1e-6 ic=5",
        Dialect::Ngspice,
        &no_diodes(),
        Some(&[0.0, 0.0, 0.0]),
        5e-5,
        TimeStep::Fixed(1e-5),
    )
    .unwrap();
    let without_ic = simulate_transient(
        "V1 a 0 10\nR1 a b 1000\nC1 b 0 1e-6",
        Dialect::Ngspice,
        &no_diodes(),
        None,
        5e-5,
        TimeStep::Fixed(1e-5),
    )
    .unwrap();

    assert_eq!(with_ic.len(), without_ic.len());
    for ((t, seeded), (_, rest)) in with_ic.iter().zip(without_ic.iter()) {
        assert_eq!(seeded.x, rest.x, "traces diverge at t = {t}");
    }
}

/// An `ic` the circuit's own equations contradict is reported, not quietly overridden. C1 is
/// wired straight across V1, whose branch equation already fixes that voltage at 10 V, and no
/// unknown in that equation is left free to absorb the 5 V difference.
#[test]
fn an_ic_contradicting_the_circuit_is_reported_not_overridden() {
    let error = simulate_transient(
        "V1 a 0 10\nR1 a 0 1000\nC1 a 0 1e-6 ic=5",
        Dialect::Ngspice,
        &no_diodes(),
        None,
        1e-5,
        TimeStep::Fixed(1e-5),
    )
    .unwrap_err();

    match &error {
        DaeError::InitialCondition(inner) => {
            let message = inner.to_string();
            assert!(
                message.contains("I(V1)"),
                "expected the violated equation to be named, got {message}"
            );
        }
        other => panic!("expected DaeError::InitialCondition, got {other:?}"),
    }
}
