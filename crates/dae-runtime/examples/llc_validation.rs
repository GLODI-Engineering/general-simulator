//! Open-loop LLC resonant converter (400V bus, half-bridge, series resonant tank, coupled-
//! inductor transformer, full-wave rectified output) -- the same specification as
//! `internal-archive`'s `experiments/converters-benchmark-open-loop-topologies`
//! (`code/llc/`), used as a cross-simulator validation and timing comparison against ngspice
//! and Xyce on the hardest topology in that experiment (two switches, a resonant tank near
//! the switching frequency, and a coupled-inductor transformer via `general-mna`'s `K` stamp
//! -- not just a single MOSFET and diode like the buck/boost cases). An example, not a `#[test]`
//! fixture: unlike buck/boost, there is no simple hand-derivable closed-form target for an
//! LLC resonant converter's steady-state output, so this is exploratory validation against
//! the other simulators' numbers, not a pass/fail assertion.
//!
//! Result recorded when this was last run (see
//! `internal-archive/experiments/elspice-pwl-boost-llc-vs-xyce-ngspice/README.md`
//! for the full writeup): avg `Vout` (last 90% of the run) = 33.70V, a ~6.3% spread against
//! ngspice/Xyce's own (tightly mutually agreeing, <0.03% apart) ~31.69V, down from ~8.7% before
//! the fix below. The fix resolves the dead-time spike itself; the remaining spread is a
//! separate, pre-existing parameter-fitting gap (see this file's PWL device comments below and
//! the README's Next Steps) unrelated to the spike.
//!
//! ## Dead-time voltage spike -- root-caused and fixed
//!
//! During dead time (both switches gated off simultaneously, twice per switching period),
//! node `vx` had essentially zero conductance to any reference from the switch stamps
//! themselves (only the resonant tank's `Cr` coupled to it at all) -- a genuinely
//! near-floating node. Under [`dae_runtime::simulate_transient_with_mosfets`]'s trapezoidal
//! integration, that near-zero damping triggered classic **trapezoidal ringing** (A-stable
//! but not L-stable: `V(vx)` oscillated between roughly +/-3000V, sign-flipping every single
//! step, while every other tracked quantity stayed smooth throughout).
//!
//! Two things fixed this, one at each level:
//! 1. **Circuit level (the actual fix)**: added the `Rs1`/`Cs1`, `Rs2`/`Cs2` RC snubbers --
//!    the *same* ones the reference Xyce/ngspice decks already have and this translation had
//!    originally dropped. A real MOSFET always has nonzero output capacitance (`Coss`); an
//!    RC snubber approximates it, giving the switching node a genuine charge-storage anchor
//!    during dead time (the capacitor) plus real damping to dissipate the resulting ringing
//!    (the resistor) -- exactly why real converters use snubbers in hardware, not just in
//!    simulation. With them in place, `V(vx)` transitions *smoothly and monotonically*
//!    between rails during dead time (a physically correct RC-discharge-shaped ramp), no
//!    oscillation at all.
//! 2. **Solver level (defense in depth, not a substitute for the above)**: `dae-runtime` now
//!    detects trapezoidal ringing directly (three consecutive values of the same unknown
//!    alternating in sign without shrinking in magnitude) and falls back to backward Euler
//!    for a short cooldown when it happens, the same way it already does for a diode-segment
//!    or gate-state change. This bounds the damage for any circuit that still lacks adequate
//!    damping at some node, but a circuit with real snubbers/parasitic capacitance (like this
//!    one, now) shouldn't need to lean on it.

use std::collections::BTreeMap;
use std::io::Write;
use std::time::Instant;

use dae_runtime::{simulate_transient_with_mosfets, GateState};
use general_spice_core::Dialect;
use pwl_devices::{Diode, Mosfet};

fn main() {
    let netlist = "V1 vin 0 400\n\
                    D1 vin vx mosfetmodel\n\
                    D2 vx 0 mosfetmodel\n\
                    Rs1 vin vx 1k\n\
                    Cs1 vin vx 1n\n\
                    Rs2 vx 0 1k\n\
                    Cs2 vx 0 1n\n\
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
        "general-simulator LLC open-loop: {} steps in {:.3}s ({:.0} steps/s)",
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

    // CSV export (t, V(vout), V(vx)) for the cross-simulator waveform plot -- see
    // internal-archive's experiments/elspice-pwl-boost-llc-vs-xyce-ngspice/README.md.
    // Same `t,unknown1,unknown2,...` column convention as general-simulator-cli's own CSV output.
    let out_path = "llc_elspice_pwl_out.csv";
    let mut f = std::fs::File::create(out_path).unwrap();
    writeln!(f, "t,V(vout),V(vx)").unwrap();
    for (t, point) in &trace {
        writeln!(
            f,
            "{t},{},{}",
            point.value("V(vout)").unwrap(),
            point.value("V(vx)").unwrap()
        )
        .unwrap();
    }
    println!("wrote {out_path}");
}
