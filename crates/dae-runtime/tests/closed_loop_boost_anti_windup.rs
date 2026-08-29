//! A closed-loop boost converter (Vin=12V, Vref=24V, L=100uH, C=100uF, R=50 ohm, 100kHz) —
//! the *same specification* as `internal-archive`'s
//! `experiments/converters-benchmark-boost-pid`, whose own conclusion states the documented
//! Xyce/ngspice attempt at this circuit never actually achieved working regulation: one-sided
//! anti-windup let the integrator wind down past recovery during the startup overshoot, PWM
//! duty floored at zero, and the converter stopped switching for the rest of the run — the
//! near-24V final reading in both simulators was just the output capacitor passively
//! discharging through the load, not real control (see that experiment's Conclusion and
//! `gotchas/xyce-boost-pi-nonlinear-failure-integrator-windup.md` in that repo).
//!
//! This test checks that `simulate_closed_loop`'s two-sided conditional-integration
//! anti-windup avoids that specific failure mode: unlike the documented case, gate switching
//! must still be active near the end of the run, and the duty command must settle near the
//! theoretical ideal continuous-conduction-mode value `D = 1 - Vin/Vref = 1 - 12/24 = 0.5`
//! (not pinned at 0 or 1).

use continuous_blocks::Pid;
use dae_runtime::{sawtooth_carrier, simulate_closed_loop, GateState};
use general_spice_core::Dialect;
use pwl_devices::{IdealDiode, IdealSwitch};
use std::collections::BTreeMap;

#[test]
fn boost_pid_regulates_without_windup_collapse() {
    let netlist =
        "V1 vin 0 12\nD1 vx 0 idealswitchmodel\nD2 vx vout dmodel\nL1 vin vx 100u\nC1 vout 0 100u\nR1 vout 0 50";
    let switch = IdealSwitch::new(0.01, IdealDiode::new(0.0, -1e6, 0.0, 1e6, 0.0));
    let mut ideal_switches = BTreeMap::new();
    ideal_switches.insert("D1".to_string(), switch);
    let mut diodes = BTreeMap::new();
    diodes.insert(
        "D2".to_string(),
        IdealDiode::new(0.0, -100.0, 0.0, 0.6, 100.0),
    );

    let reference = 24.0;
    // Kp small enough that proportional action alone doesn't saturate duty from the very
    // first step (error(0) = 24, so Kp*24 must stay well under 1): a real, non-obvious tuning
    // constraint for this specific circuit found while building this fixture -- with a
    // saturating actuator, "too aggressive Kp" and "windup" are two distinct failure modes,
    // and anti-windup alone does not fix the former.
    let pid = Pid::new(0.01, 20.0, 0.0, 1000.0).unwrap();
    let controller = pid.to_state_space();

    let switching_freq = 100_000.0;
    let measure = |point: &dae_runtime::OperatingPoint| point.value("V(vout)").unwrap();
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

    let dt = 1e-7;
    let t_final = 0.010; // 5 RC time constants (R*C = 50 * 100uF = 5ms), well past startup

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
        0.01,
        None,
        t_final,
        dt,
    )
    .unwrap();

    // The specific failure this test guards against: duty permanently pinned at 0 (or 1),
    // meaning the converter stopped actively switching. Check the last 200 steps (2 switching
    // periods) all show a genuine, non-saturated duty command.
    let tail_end = &trace[trace.len() - 200..];
    let all_actively_switching = tail_end
        .iter()
        .all(|(_, _, duty)| duty.clamp(0.0, 1.0) > 0.02 && duty.clamp(0.0, 1.0) < 0.98);
    assert!(
        all_actively_switching,
        "converter must still be actively switching near t_final, not pinned at a saturation rail \
         (this is the exact failure mode the documented Xyce/ngspice attempt exhibited)"
    );

    // Second half average duty should be close to the theoretical CCM value D = 1 - Vin/Vref.
    let tail = &trace[trace.len() / 2..];
    let avg_duty: f64 =
        tail.iter().map(|(_, _, d)| d.clamp(0.0, 1.0)).sum::<f64>() / tail.len() as f64;
    let expected_duty = 1.0 - 12.0 / 24.0;
    assert!(
        (avg_duty - expected_duty).abs() < 0.05,
        "avg duty (2nd half) = {avg_duty}, expected close to the CCM ideal {expected_duty}"
    );

    // And the output itself should be regulating near the reference, not stuck near 0V (the
    // documented failure's actual behavior once switching stopped) or diverging.
    let avg_vout: f64 = tail
        .iter()
        .map(|(_, point, _)| point.value("V(vout)").unwrap())
        .sum::<f64>()
        / tail.len() as f64;
    assert!(
        (avg_vout - reference).abs() < 2.0,
        "avg Vout (2nd half) = {avg_vout}, expected close to reference = {reference}"
    );
}
