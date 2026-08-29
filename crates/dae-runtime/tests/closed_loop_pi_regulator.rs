//! A real closed-loop example: a PI controller (compiled via `continuous_blocks::Pid`)
//! regulates a chopper-fed RC circuit's output voltage to a reference via PWM, using
//! `dae_runtime::simulate_closed_loop`. This is the payoff described in this project's
//! original plan — a PID controller driving a switching ideal switch gate with no Xyce-style
//! `tanh`-smoothed comparator needed, since gate switching here is just another LCP-resolved
//! mode, never a Newton-Raphson convergence hazard.
//!
//! Circuit: `V1 (10V) -- D1 (ideal switch, drain=a, source=b) -- Rload (10 ohm) -- Cfilt (100uF) --
//! ground`, `Rload`/`Cfilt` both directly at node `b` (the switched node), driven by a 10 kHz
//! PWM carrier. Note this is *not* a clean `V_out ~= duty * V_in` relationship: with no
//! inductor, and `Rload` comparable to `Ron`, the averaged relationship is actually
//! `V_out ~= duty * V_in / (duty + Ron/Rload)` (worked out by hand while debugging this
//! fixture — an early version of this test assumed the naive `duty * V_in` relationship,
//! wrongly, and used it only to sanity-check that the reference was in a *reachable* range,
//! never as the pass/fail criterion itself).
//!
//! ## What's verified, and what isn't
//!
//! A full closed-form solution for a real closed-loop switching converter's exact transient
//! (every individual switching ripple included) is not analytically tractable by hand — that
//! is not a gap specific to this project, it's true of switching converters in general, which
//! is exactly why simulators like this one exist. What *is* checked, and is a standard,
//! well-established control-theory property rather than a guess: integral control drives
//! steady-state error to zero for a step reference. In practice this sampled-data loop
//! (measuring the rippling PWM node directly, at a fixed `dt` rather than synchronously with
//! the switching period) settles into a bounded oscillation around the reference rather than a
//! tight DC value — expected for this setup, not a bug — so this test averages over a wide tail
//! window (half the simulated run) and uses a tolerance sized for that ripple, not for
//! millivolt-level settling.

use continuous_blocks::Pid;
use dae_runtime::{sawtooth_carrier, simulate_closed_loop, GateState};
use general_spice_core::Dialect;
use pwl_devices::{IdealDiode, IdealSwitch};
use std::collections::BTreeMap;

#[test]
fn pi_controller_regulates_output_to_reference_at_steady_state() {
    // D1 declared (drain=a, source=b): the body diode's real forward direction (anode=source=
    // b, cathode=drain=a, i.e. b->a) must not match the normal charging direction (a->b), or
    // it would keep conducting even with the gate off, defeating the switch (see this crate's
    // journal for the full account of this bug).
    let netlist = "V1 a 0 10\nD1 a b idealswitchmodel\nRload b 0 10\nCfilt b 0 100u";
    let switch = IdealSwitch::new(0.1, IdealDiode::new(0.0, -100.0, 0.0, 0.5, 5.0));
    let mut ideal_switches = BTreeMap::new();
    ideal_switches.insert("D1".to_string(), switch);
    let diodes = BTreeMap::new();

    let reference = 6.0;
    let pid = Pid::new(0.05, 50.0, 0.0, 1000.0).unwrap();
    let controller = pid.to_state_space();

    let switching_freq = 10_000.0;
    let measure = |point: &dae_runtime::OperatingPoint| point.value("V(b)").unwrap();
    let pwm = move |duty: f64, t: f64| {
        let duty = duty.clamp(0.0, 1.0);
        let carrier = sawtooth_carrier(t, switching_freq);
        let state = if carrier < duty {
            GateState::On
        } else {
            GateState::Off
        };
        BTreeMap::from([("D1".to_string(), state)])
    };

    let dt = 1.0 / switching_freq / 20.0; // 20 samples per switching period
    let t_final = 0.02; // 200 switching periods

    let trace = simulate_closed_loop(
        netlist,
        Dialect::Ngspice,
        &diodes,
        &ideal_switches,
        &controller,
        move |_| reference,
        measure,
        pwm,
        (0.0, 1.0),
        0.1,
        None,
        t_final,
        dt,
    )
    .unwrap();

    // Average over the second half of the run (well past any initial transient) rather than
    // just the last couple of periods, since this sampled-data loop settles into a bounded
    // oscillation rather than a tight DC value (see the module doc above) -- a wide window
    // averages that out into a meaningful steady-state estimate.
    let tail = &trace[trace.len() / 2..];
    let average_output: f64 = tail
        .iter()
        .map(|(_, point, _)| point.value("V(b)").unwrap())
        .sum::<f64>()
        / tail.len() as f64;

    assert!(
        (average_output - reference).abs() < 0.3,
        "average output over the second half of the run = {average_output}, expected close to reference = {reference}"
    );

    // Sanity: the duty command should have settled to a real, non-saturated regulation point,
    // not pinned at 0 or 1 (which would indicate the loop never actually regulated and the
    // reference was only reached by coincidence).
    let average_duty: f64 = tail
        .iter()
        .map(|(_, _, duty)| duty.clamp(0.0, 1.0))
        .sum::<f64>()
        / tail.len() as f64;
    assert!(
        (0.02..0.95).contains(&average_duty),
        "average duty command {average_duty} should be a real, non-saturated regulation point"
    );
}
