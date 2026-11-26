//! The other side of the ringing detector (#116): what it must **not** do.
//!
//! An offset-invariant detector sees a signature that every smooth oscillation also shows at
//! its turning points, and falling back to backward Euler there would quietly cost the run
//! its second order and its amplitude. Both are measured here on a circuit that oscillates
//! about a rail and never crosses zero — a lossless $LC$ tank across a $400\text{V}$ source,
//! seeded off its own equilibrium — so a detector blind to the offset and a detector that
//! fires on the offset are distinguished by these numbers rather than by inspection.
//!
//! $L = 1\text{mH}$, $C = 1\mu\text{F}$ give $\omega = 1/\sqrt{LC} = 31622.7766\ \text{rad/s}$
//! and $T = 198.692\mu\text{s}$. With $v_C(0) = 600\text{V}$ (the netlist's own `ic=`) and
//! $i_L(0) = 0$, the exact solution is
//!
//! $$v_C(t) = 400 + 200\cos(\omega t)$$
//!
//! which is lossless: its envelope is $200\text{V}$ for ever. Trapezoidal integration
//! reproduces that — its amplification factor for this mode has modulus exactly one — so any
//! amplitude the run loses after the startup steps is amplitude a backward-Euler fallback took.

use dae_runtime::{simulate_transient, TimeStep};
use general_spice_core::Dialect;
use pwl_devices::IdealDiode;
use std::collections::BTreeMap;

const NETLIST: &str = "V1 a 0 400\nL1 a b 1e-3\nC1 b 0 1e-6 ic=600";
const OMEGA: f64 = 31_622.776_601_683_793;
const PERIOD: f64 = std::f64::consts::TAU / OMEGA;

fn run(dt: f64, t_final: f64) -> Vec<(f64, f64)> {
    let diodes: BTreeMap<String, IdealDiode> = BTreeMap::new();
    simulate_transient(
        NETLIST,
        Dialect::Ngspice,
        &diodes,
        None,
        t_final,
        TimeStep::Fixed(dt),
    )
    .unwrap()
    .into_iter()
    .map(|(t, point)| (t, point.value("V(b)").unwrap()))
    .collect()
}

/// Peak-to-peak of `V(b)` over the samples in `[from, to)`.
fn peak_to_peak(trace: &[(f64, f64)], from: f64, to: f64) -> f64 {
    let window: Vec<f64> = trace
        .iter()
        .filter(|(t, _)| *t >= from && *t < to)
        .map(|(_, v)| *v)
        .collect();
    assert!(!window.is_empty(), "empty window {from}..{to}");
    let max = window.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let min = window.iter().cloned().fold(f64::INFINITY, f64::min);
    max - min
}

/// The oscillation is lossless, so its sixth cycle must have the same amplitude as its first.
///
/// This is the sensitive measurement, because a backward-Euler step damps this mode by
/// $1 - |1/(1 + j\omega\Delta t)| \approx (\omega\Delta t)^2/2$, which at the deliberately
/// coarse $\Delta t = T/33$ used here is about 1.8% *per step*. A detector that fires at the
/// waveform's own extrema would take one fallback step plus its three cooldown steps twice a
/// cycle — about 14% of the amplitude per cycle, over 50% across this run — while the four
/// backward-Euler steps the run legitimately starts with cost their 7% once and never again.
#[test]
fn a_lossless_tank_ringing_about_a_rail_keeps_its_amplitude() {
    let dt = 6e-6;
    let trace = run(dt, 6.5 * PERIOD);
    let first = peak_to_peak(&trace, 0.1 * PERIOD, 1.1 * PERIOD);
    let sixth = peak_to_peak(&trace, 5.1 * PERIOD, 6.1 * PERIOD);
    let lost = (first - sixth) / first;
    assert!(
        lost.abs() < 0.03,
        "amplitude went from {first} V to {sixth} V over five cycles ({:.1}% lost); a lossless \
         LC tank under trapezoidal integration keeps its amplitude, so this is backward-Euler \
         damping from a ringing fallback firing on a smooth waveform",
        lost * 100.0
    );
}

/// And the order itself, measured the way `scheme_tests` measures it — halve $\Delta t$ and
/// watch the error against the closed form quarter — but through the real transient loop, so
/// the scheme the loop actually chose is what is being graded.
#[test]
fn the_loop_stays_second_order_on_a_waveform_that_oscillates_about_a_rail() {
    let t_final = 3.0 * PERIOD;
    let exact = 400.0 + 200.0 * (OMEGA * t_final).cos();
    let error_at = |dt: f64| {
        let trace = run(dt, t_final);
        let (t, v) = *trace.last().unwrap();
        assert!(
            (t - t_final).abs() < dt,
            "run stopped at {t}, wanted {t_final}"
        );
        (v - exact).abs()
    };
    let coarse = error_at(3e-6);
    let fine = error_at(1.5e-6);
    let ratio = coarse / fine;
    assert!(
        (3.0..=5.5).contains(&ratio),
        "error ratio (dt vs dt/2) = {ratio} ({coarse} V vs {fine} V), expected close to 4.0 \
         (second order); a value near 2.0 means the loop spent the run on backward Euler"
    );
}
