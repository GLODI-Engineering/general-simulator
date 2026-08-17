//! Open-loop LLC resonant converter (400V bus, half-bridge, series resonant tank, coupled-
//! inductor transformer, full-wave rectified output) -- the same specification as
//! `internal-archive`'s `experiments/converters-benchmark-open-loop-topologies`
//! (`code/llc/`), used as a cross-simulator validation and timing comparison against ngspice
//! and Xyce on the hardest topology in that experiment (two switches, a resonant tank near
//! the switching frequency, and a coupled-inductor transformer via `elspice-mna`'s `K` stamp
//! -- not just a single MOSFET and diode like the buck/boost cases). An example, not a `#[test]`
//! fixture: unlike buck/boost, there is no simple hand-derivable closed-form target for an
//! LLC resonant converter's steady-state output, so this is exploratory validation against
//! the other simulators' numbers, not a pass/fail assertion.
//!
//! Result recorded when this was last run (see
//! `internal-archive/experiments/elspice-pwl-buck-vs-xyce-ngspice/README.md`'s
//! follow-up entry for the full writeup): avg `Vout` (last 90% of the run) landed within
//! ~9% of ngspice's own steady-state average -- a real topology-complexity increase over
//! buck/boost's <1% spread, plausible given a resonant tank is far more sensitive to exact
//! component/parasitic modeling than a simple hard-switched filter, but still the right
//! ballpark, not a different regime.
//!
//! One genuine numerical finding worth being upfront about: at the exact instant both
//! switches are in dead time (neither gated on), node `vx` has essentially zero conductance
//! to any reference from the switch stamps themselves (only the resonant tank's `Cr`
//! provides any path at all), and the backward-Euler step forced right at that gate
//! transition produces a large (thousands of volts) one-step spike in `V(vx)` before
//! immediately correcting on the next step. `V(vout)` itself is not visibly perturbed by
//! it (checked directly, not assumed). A tiny nonzero `g_off` leakage on the switch's body
//! diode reduces the spike's magnitude but does not eliminate it, so this is not treated as
//! solved here -- it's recorded as a genuine known limitation for very low-conductance
//! dead-time nodes, worth investigating further in a future session.

use std::collections::BTreeMap;
use std::time::Instant;

use dae_runtime::{simulate_transient_with_mosfets, GateState};
use pwl_devices::{Diode, Mosfet};
use spice_core::Dialect;

fn main() {
    let netlist = "V1 vin 0 400\n\
                    D1 vin vx mosfetmodel\n\
                    D2 vx 0 mosfetmodel\n\
                    Cr vx vr 22n\n\
                    Lr vr vp 100u\n\
                    Lm vp 0 400u\n\
                    Lpri vp 0 1000u\n\
                    Lsec1 vy 0 10u\n\
                    Lsec2 0 vz 10u\n\
                    K1 Lpri Lsec1 0.99\n\
                    K2 Lpri Lsec2 0.99\n\
                    K3 Lsec1 Lsec2 0.99\n\
                    D3 vy vout dmodel\n\
                    D4 vz vout dmodel\n\
                    Cout vout 0 470u\n\
                    Rout vout 0 20";

    // D1/D2 stand in for the ideal half-bridge switches S1/S2 (RON=10m, no body diode of
    // their own in the reference decks): body-diode threshold set unreachably high so
    // gate=off means true full blocking, with a tiny nonzero g_off (see module doc, the
    // dead-time spike finding).
    let ideal_switch = Mosfet::new(0.01, Diode::new(0.0, -1e6, 1e-6, 1e6, 0.0));
    let mut mosfets = BTreeMap::new();
    mosfets.insert("D1".to_string(), ideal_switch);
    mosfets.insert("D2".to_string(), ideal_switch);
    // D3/D4 are the real rectifier diodes (D_IDEAL: IS=1e-14, N=1, RS=10m); g_on=1/RS=100.
    let mut diodes = BTreeMap::new();
    diodes.insert("D3".to_string(), Diode::new(0.0, -100.0, 0.0, 0.6, 100.0));
    diodes.insert("D4".to_string(), Diode::new(0.0, -100.0, 0.0, 0.6, 100.0));

    // Matches the Xyce/ngspice decks' PULSE sources: S1 (D1) on for the first 4.8us of each
    // 10us period; S2 (D2) on for the complementary window, phase-shifted by 5us -- ~0.2us
    // dead time on each edge.
    let period = 10e-6;
    let on_width = 4.8e-6;
    let gate_signal = move |name: &str, t: f64| {
        let phase_offset = if name == "D2" { 5e-6 } else { 0.0 };
        let tau = (t - phase_offset).rem_euclid(period);
        if tau < on_width {
            GateState::On
        } else {
            GateState::Off
        }
    };

    let dt = 1e-8; // matches the .tran 10n step in both reference decks
    let t_final = 1e-3; // 100 switching periods, matches both reference decks

    let start = Instant::now();
    let trace = simulate_transient_with_mosfets(
        netlist,
        Dialect::Ngspice,
        &diodes,
        &mosfets,
        gate_signal,
        0.01,
        None,
        t_final,
        dt,
    )
    .unwrap();
    let elapsed = start.elapsed();

    println!(
        "elspice-pwl LLC open-loop: {} steps in {:.3}s ({:.0} steps/s)",
        trace.len(),
        elapsed.as_secs_f64(),
        trace.len() as f64 / elapsed.as_secs_f64()
    );

    let tail = &trace[trace.len() / 10..]; // last 90% (skip only the initial transient)
    let avg_vout: f64 = tail
        .iter()
        .map(|(_, p)| p.value("V(vout)").unwrap())
        .sum::<f64>()
        / tail.len() as f64;
    println!("avg Vout (last 90%): {avg_vout:.4}");
}
