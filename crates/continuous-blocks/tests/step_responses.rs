//! Simulates each block's compiled `StateSpace` with RK4 against a step input and compares
//! against a closed-form response worked out by hand first — the same "derive it
//! independently, then check the code reproduces it" discipline as the rest of this project's
//! milestones (see `pwl-devices/tests/two_diode_circuit.rs`, `dae-runtime/tests/mosfet.rs`).

use continuous_blocks::{Pid, TransferFunction};

fn simulate_step(ss: &continuous_blocks::StateSpace, t_final: f64, dt: f64) -> f64 {
    let mut x = vec![0.0; ss.states()];
    let u = [1.0];
    let steps = (t_final / dt).round() as usize;
    for _ in 0..steps {
        x = ss.rk4_step(&x, &u, dt);
    }
    ss.output(&x, &u)[0]
}

/// 1/(s+1) (RC lowpass, tau=1): dx/dt = -x + u, x(0)=0, u=1 for t>=0 => x(t) = 1 - e^(-t),
/// y = x. At t=1: y = 1 - e^-1 ~= 0.6321205588.
#[test]
fn first_order_lowpass_step_response_matches_analytic_solution() {
    let ss = TransferFunction::new(vec![1.0], vec![1.0, 1.0])
        .unwrap()
        .to_state_space();
    let y = simulate_step(&ss, 1.0, 1e-3);
    let expected = 1.0 - (-1.0_f64).exp();
    assert!(
        (y - expected).abs() < 1e-6,
        "y(1) = {y}, expected {expected}"
    );
}

/// dx/dt = u, x(0)=0, u=1 for t>=0 => x(t) = t exactly (a pure integrator's step response is
/// a ramp) -- RK4 is exact for a constant derivative, so this should match to floating-point
/// precision, not just approximately.
#[test]
fn integrator_step_response_is_an_exact_ramp() {
    let ss = TransferFunction::new(vec![1.0], vec![1.0, 0.0])
        .unwrap()
        .to_state_space();
    let y = simulate_step(&ss, 2.5, 1e-3);
    assert!((y - 2.5).abs() < 1e-9, "y(2.5) = {y}, expected 2.5 exactly");
}

/// The exact-cancellation PID case derived by hand in `src/pid.rs`'s doc comment: with Kd=0,
/// the filter pole at -N cancels exactly against a zero of the numerator, so the closed-loop
/// step response of the *full* (non-minimal, 2-state) realization equals the ideal PI response
/// y(t) = Kp + Ki*t for every t, not just asymptotically. Checked at several t, not just one,
/// since an indexing bug in the general n=2 canonical-form code could easily produce a
/// response that's only right at t=0 or only in the limit.
#[test]
fn pid_with_kd_zero_reproduces_exact_pi_ramp_response() {
    let pid = Pid::new(2.0, 3.0, 0.0, 10.0).unwrap();
    let ss = pid.to_state_space();
    for &t in &[0.1, 0.5, 1.0, 2.0] {
        let y = simulate_step(&ss, t, 1e-4);
        let expected = 2.0 + 3.0 * t;
        assert!(
            (y - expected).abs() < 1e-4,
            "y({t}) = {y}, expected {expected} (Kp + Ki*t)"
        );
    }
}

/// A nonzero Kd changes the transient but not the DC/ramp trend: the response should still
/// grow roughly linearly at large t (the derivative term's effect decays), and in particular
/// should not blow up or go unstable for a well-damped filter pole (N=10 here, well inside the
/// simulated timescale) -- a coarse sanity check that a real (not just Kd=0-cancelling) PID
/// realization is at least stable and roughly PI-like at large t, since deriving its exact
/// transient by hand is much more involved than the Kd=0 case.
#[test]
fn pid_with_nonzero_kd_stays_bounded_and_trends_like_pi_at_large_t() {
    let pid = Pid::new(2.0, 3.0, 0.5, 10.0).unwrap();
    let ss = pid.to_state_space();
    let y_early = simulate_step(&ss, 0.05, 1e-5);
    let y_late = simulate_step(&ss, 3.0, 1e-4);
    assert!(y_early.is_finite() && y_late.is_finite());
    let expected_late = 2.0 + 3.0 * 3.0; // filter transient should have decayed by t=3 (N=10)
    assert!(
        (y_late - expected_late).abs() < 0.05,
        "y(3) = {y_late}, expected close to {expected_late}"
    );
}
