//! A signal-domain block's own `ic=` / `y0=`, end to end: netlist text through
//! `general_mna::build_system` and the real block-graph evaluation loop.
//!
//! Every expected number is a closed form derived by hand from the block's own differential
//! (or difference) equation, never read off a previous run, and each case is chosen so that a
//! start from rest lands somewhere visibly different -- none of these can pass by accident.

use std::collections::BTreeMap;

use dae_runtime::{simulate_transient_with_blocks, DaeError, SignalValue, TimeStep};
use general_mna::{build_system, System};
use general_spice_core::Dialect;

const CIRCUIT: &str = "V1 a 0 5\nR1 a 0 1k\n";

fn run(blocks_text: &str, t_final: f64, dt: f64) -> Vec<(f64, BTreeMap<String, SignalValue>)> {
    let source = format!("{CIRCUIT}{blocks_text}");
    let System {
        ideal_diodes,
        ideal_switches,
        gates,
        blocks,
        shared_r_on,
        ..
    } = build_system(&source, Dialect::Ngspice).unwrap();
    simulate_transient_with_blocks(
        &source,
        Dialect::Ngspice,
        &ideal_diodes,
        &ideal_switches,
        &blocks,
        &gates,
        shared_r_on,
        None,
        t_final,
        TimeStep::Fixed(dt),
    )
    .unwrap()
    .into_iter()
    .map(|(t, _, outputs)| (t, outputs))
    .collect()
}

fn scalar(outputs: &BTreeMap<String, SignalValue>, name: &str) -> f64 {
    outputs[name].as_scalar().unwrap()
}

/// $\dot{x} = -x + u$, $u = 1$, $x(0) = 0.5$: $y(t) = 1 - 0.5\,e^{-t}$. From rest it would be
/// $1 - e^{-t}$, i.e. $0.00995$ instead of $0.50498$ after the first $10$ ms step. RK4's own
/// error on this step is about $h^5/120 \approx 10^{-12}$.
#[test]
fn tf_ic_is_the_canonical_state() {
    let trace = run(
        "U kind=const value=1\nG kind=tf in=U num=[1] den=[1,1] ic=[0.5]\n",
        0.05,
        0.01,
    );
    for (t, outputs) in &trace {
        let expected = 1.0 - 0.5 * (-t).exp();
        let y = scalar(outputs, "G");
        assert!((y - expected).abs() < 1e-10, "t={t}: {y} vs {expected}");
    }
}

/// $(s+3)/(s^2+3s+2)$ has $G(0) = 1.5$; settled at $y_0 = 6$ and fed the matching $u = 4$ it is
/// at an equilibrium and must not move at all.
#[test]
fn tf_y0_with_the_matching_input_stays_put() {
    let trace = run(
        "U kind=const value=4\nG kind=tf in=U num=[1,3] den=[1,3,2] y0=6\n",
        0.1,
        0.01,
    );
    for (t, outputs) in &trace {
        let y = scalar(outputs, "G");
        assert!((y - 6.0).abs() < 1e-12, "t={t}: {y}");
    }
}

/// An undriven harmonic oscillator, $\ddot{y} = -y$, started at $(1, 0)$: $y(t) = \cos t$.
/// From rest it never leaves zero.
#[test]
fn statespace_ic_is_the_state_vector() {
    let trace = run(
        "U kind=const value=0\n\
         G kind=statespace in=U a=[[0,1],[-1,0]] b=[0,0] c=[1,0] ic=[1,0]\n",
        0.05,
        0.01,
    );
    for (t, outputs) in &trace {
        let y = scalar(outputs, "G");
        assert!((y - t.cos()).abs() < 1e-10, "t={t}: {y}");
    }
}

/// A PI ($k_d = 0$) pre-loaded to $0.6$ and fed a constant error $e = 0.1$:
/// $y(t) = 0.6 + k_p e + k_i e\,t = 0.8 + 0.3\,t$. The derivative filter's pole cancels exactly
/// analytically; RK4 resolves it to about $(10h)^5/120 \approx 10^{-12}$ at $h = 1$ ms.
#[test]
fn pid_ic_preloads_the_integrator() {
    let trace = run(
        "E kind=const value=0.1\n\
         C kind=pid in=E kp=2 ki=3 kd=0 n=10 clamp_lo=-5 clamp_hi=5 ic=0.6\n",
        0.005,
        0.001,
    );
    for (t, outputs) in &trace {
        let expected = 0.8 + 0.3 * t;
        let y = scalar(outputs, "C");
        assert!((y - expected).abs() < 1e-9, "t={t}: {y} vs {expected}");
    }
}

/// $x[k+1] = 0.5\,x[k]$, $y = x$, $x[0] = 8$: the output read after the $k$-th update is
/// $8 \cdot 0.5^k$ -- $4, 2, 1, \dots$ rather than zero forever.
#[test]
fn discretestatespace_ic_is_the_state_vector() {
    let trace = run(
        "U kind=const value=0\n\
         G kind=discretestatespace in=U a=[[0.5]] b=[0] c=[1] ts=0.01 ic=8\n",
        0.03,
        0.01,
    );
    let ys: Vec<f64> = trace.iter().map(|(_, o)| scalar(o, "G")).collect();
    assert_eq!(&ys[..3], &[4.0, 2.0, 1.0]);
}

/// $H(z) = (0.5z + 0.25)/(z^2 - 0.5z)$, $H(1) = 1.5$: settled at $y_0 = 3$ with $u = 2$ is a
/// fixed point of the difference equation.
#[test]
fn discretetf_y0_with_the_matching_input_stays_put() {
    let trace = run(
        "U kind=const value=2\n\
         G kind=discretetf in=U num=[0.5,0.25] den=[1,-0.5,0] ts=0.01 y0=3\n",
        0.05,
        0.01,
    );
    for (t, outputs) in &trace {
        assert_eq!(scalar(outputs, "G"), 3.0, "t={t}");
    }
}

/// Backward-Euler integrator only ($k_p = k_d = 0$, $k_i = 2$, $T = 0.01$), pre-loaded to an
/// output of $0.6$, constant error $0.5$: $y[k] = 0.6 + k_i T e\,k = 0.6 + 0.01\,k$.
#[test]
fn discretepid_ic_preloads_the_integrator() {
    let trace = run(
        "E kind=const value=0.5\n\
         C kind=discretepid in=E kp=0 ki=2 kd=0 n=1 ts=0.01 clamp_lo=-5 clamp_hi=5 \
         integration_method=backward ic=0.6\n",
        0.03,
        0.01,
    );
    for (k, (_, outputs)) in trace.iter().enumerate() {
        let expected = 0.6 + 0.01 * (k as f64 + 1.0);
        let y = scalar(outputs, "C");
        assert!((y - expected).abs() < 1e-12, "k={k}: {y} vs {expected}");
    }
}

/// A 100 Hz oscillator started a quarter-cycle in: $\phi = 0.25 + 100\,t$ (mod 1).
#[test]
fn vco_ic_is_the_initial_phase() {
    let trace = run(
        "F kind=const value=100\nO kind=vco in=F f_min=1 f_max=1000 ic=0.25\n",
        0.003,
        0.001,
    );
    for (t, outputs) in &trace {
        let expected = (0.25 + 100.0 * t).rem_euclid(1.0);
        let phase = scalar(outputs, "O");
        assert!((phase - expected).abs() < 1e-12, "t={t}: {phase}");
    }
}

/// Every logic state holds its declared value while nothing drives it: a latch with neither
/// input asserted, a flip-flop and a counter with no clock edge, a hysteresis comparator whose
/// input sits inside its own band. From rest all four read zero.
#[test]
fn logic_states_and_the_counter_start_where_declared() {
    let trace = run(
        "Z kind=const value=0\nMID kind=const value=0.5\n\
         Q kind=srlatch set=Z reset=Z ic=1\n\
         D kind=dff clk=Z d=Z ic=1\n\
         N kind=counter clk=Z ic=7\n\
         H kind=hysteresis in=MID high=1 low=0 ic=1\n",
        0.003,
        0.001,
    );
    for (_, outputs) in &trace {
        assert_eq!(scalar(outputs, "Q"), 1.0);
        assert_eq!(scalar(outputs, "D"), 1.0);
        assert_eq!(scalar(outputs, "N"), 7.0);
        assert_eq!(scalar(outputs, "H"), 1.0);
    }
}

/// No magnet flux, no currents, no voltages, no friction, no load: nothing produces torque or
/// back-EMF, so the rotor coasts at its declared $\omega_m = 100$ rad/s and
/// $\theta_e(t) = \theta_0 + p\,\omega_m t = 0.5 + 400\,t$.
#[test]
fn pmsm_ic_is_the_four_machine_states() {
    let trace = run(
        "Z kind=const value=0\n\
         M kind=pmsm r_s=0.5 l_d=1e-3 l_q=1e-3 lambda_pm=0 pole_pairs=4 inertia=1e-5 friction=0 \
         inputs=Z,Z,Z outputs=ID,IQ,WM,TH ic=[0,0,100,0.5]\n",
        0.003,
        0.001,
    );
    for (t, outputs) in &trace {
        assert_eq!(scalar(outputs, "WM"), 100.0);
        let theta = scalar(outputs, "TH");
        assert!((theta - (0.5 + 400.0 * t)).abs() < 1e-12, "t={t}: {theta}");
    }
}

/// A hand-built block bypasses the netlist parser's own length check; the runtime still refuses
/// it rather than indexing past the end or silently truncating.
#[test]
fn a_hand_built_ic_of_the_wrong_length_is_refused() {
    let System { mut blocks, .. } = build_system(
        "U kind=const value=1\nG kind=tf in=U num=[1] den=[1,1]\n",
        Dialect::Ngspice,
    )
    .unwrap();
    blocks.iter_mut().find(|b| b.name == "G").unwrap().ic = Some(vec![1.0, 2.0]);
    let err = simulate_transient_with_blocks(
        CIRCUIT,
        Dialect::Ngspice,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &blocks,
        &BTreeMap::new(),
        0.1,
        None,
        0.01,
        TimeStep::Fixed(0.01),
    )
    .unwrap_err();
    assert!(matches!(
        err,
        DaeError::BlockInitialConditionLength {
            expected: 1,
            got: 2,
            ..
        }
    ));
}
