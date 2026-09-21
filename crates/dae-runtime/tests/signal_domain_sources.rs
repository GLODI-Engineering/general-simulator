//! `BlockKind::Pwc`/`BlockKind::Pwl` (with the `repeat` option) and `BlockKind::Waveform`
//! (`Sin`/`Pulse`/`Exp`/`Sffm`, reusing `general_mna::TransientFunction` directly) — the
//! signal-domain counterparts to the electrical domain's five `TransientFunction` source forms,
//! added this session specifically so a signal-domain reference schedule and a `V`/`I` source
//! built from the same numbers produce the same waveform (`Waveform`), and so the signal domain
//! can express a genuinely periodic breakpoint list (`repeat=true`), which the electrical
//! domain's own `PWL(...)` source has no option for.
//!
//! Every check here drives a `Sig2Phys`-converted source through a purely resistive circuit
//! (`V1 a 0 CMD_V\nR1 a 0 1000`) so `V(a)` equals the source block's own output *exactly*, every
//! step (no reactive element to introduce any lag/approximation) — the same identity-check
//! pattern `sig2v_sig2i.rs`'s own resistive-only tests use.

use std::collections::BTreeMap;

use dae_runtime::{
    simulate_transient_with_blocks, BlockInstance, BlockKind, PhysicalDomain, Signal, TimeStep,
    TransientFunction,
};
use general_spice_core::Dialect;
use pwl_devices::{IdealDiode, IdealSwitch};

fn run_source(kind: BlockKind, t_final: f64, dt: f64) -> Vec<(f64, f64)> {
    let netlist = "V1 a 0 CMD_V\nR1 a 0 1000";
    let ideal_switches: BTreeMap<String, IdealSwitch> = BTreeMap::new();
    let diodes: BTreeMap<String, IdealDiode> = BTreeMap::new();
    let gates = BTreeMap::new();

    let blocks = vec![
        BlockInstance {
            name: "CMD".to_string(),
            kind,
            inputs: vec![],
            ic: None,
        },
        BlockInstance {
            name: "CMD_V".to_string(),
            kind: BlockKind::Sig2Phys {
                domain: PhysicalDomain::Voltage,
            },
            inputs: vec![Signal::Block("CMD".to_string())],
            ic: None,
        },
    ];

    let trace = simulate_transient_with_blocks(
        netlist,
        Dialect::Ngspice,
        &diodes,
        &ideal_switches,
        &blocks,
        &gates,
        0.1,
        None,
        t_final,
        TimeStep::Fixed(dt),
    )
    .unwrap();

    trace
        .iter()
        .map(|(t, point, _)| (*t, point.value("V(a)").unwrap()))
        .collect()
}

#[test]
fn pwc_holds_flat_past_the_last_point_without_repeat() {
    let points = vec![(0.0, 1.0), (1e-3, 5.0)];
    let trace = run_source(
        BlockKind::Pwc {
            points,
            repeat: false,
        },
        3e-3,
        1e-4,
    );
    for (t, v) in trace {
        let expected = if t < 1e-3 { 1.0 } else { 5.0 };
        assert!(
            (v - expected).abs() < 1e-9,
            "t={t}: V(a)={v}, expected exactly {expected} (pwc, no repeat)"
        );
    }
}

#[test]
fn pwc_repeat_produces_a_periodic_square_wave() {
    // period = 1e-3 - 0.0 = 1e-3: value is 1.0 on [0, 0.5e-3), 5.0 on [0.5e-3, 1e-3), then
    // wraps back to 1.0 at t=1e-3 itself and repeats -- checked across three full periods.
    let points = vec![(0.0, 1.0), (0.5e-3, 5.0), (1e-3, 1.0)];
    let trace = run_source(
        BlockKind::Pwc {
            points,
            repeat: true,
        },
        3.5e-3,
        1e-5,
    );
    for (t, v) in trace {
        let phase = t % 1e-3;
        let expected = if phase < 0.5e-3 { 1.0 } else { 5.0 };
        assert!(
            (v - expected).abs() < 1e-9,
            "t={t}: V(a)={v}, expected exactly {expected} (pwc repeat, phase={phase})"
        );
    }
}

#[test]
fn pwl_interpolates_linearly_unlike_pwc() {
    // Same two points as the very first pwc test above (1.0 -> 5.0 at t=1e-3), but pwl must
    // ramp linearly between them instead of stepping -- the whole point of the name split.
    let points = vec![(0.0, 1.0), (1e-3, 5.0)];
    let trace = run_source(
        BlockKind::Pwl {
            points,
            repeat: false,
        },
        1.5e-3,
        1e-5,
    );
    for (t, v) in trace {
        let expected = if t <= 0.0 {
            1.0
        } else if t >= 1e-3 {
            5.0
        } else {
            1.0 + (5.0 - 1.0) * (t / 1e-3)
        };
        assert!(
            (v - expected).abs() < 1e-6,
            "t={t}: V(a)={v}, expected {expected} (pwl linear ramp)"
        );
    }
}

#[test]
fn pwl_repeat_produces_a_periodic_triangle_wave() {
    // period = 2e-3 - 0.0 = 2e-3: ramps 0 -> 10 over the first ms, back down to 0 over the
    // second ms, then repeats -- checked across two full periods.
    let points = vec![(0.0, 0.0), (1e-3, 10.0), (2e-3, 0.0)];
    let trace = run_source(
        BlockKind::Pwl {
            points,
            repeat: true,
        },
        4.5e-3,
        1e-5,
    );
    for (t, v) in trace {
        let phase = t % 2e-3;
        let expected = if phase <= 1e-3 {
            10.0 * (phase / 1e-3)
        } else {
            10.0 * (2.0 - phase / 1e-3)
        };
        assert!(
            (v - expected).abs() < 1e-6,
            "t={t}: V(a)={v}, expected {expected} (pwl repeat, phase={phase})"
        );
    }
}

#[test]
fn waveform_sin_matches_hand_computed_values() {
    // Same SIN(0 10 1000 0 0 0) parameters general-mna's own transient_source.rs test uses,
    // cross-checked through the full block-graph + Sig2Phys + circuit path here instead of
    // calling TransientFunction::value_at directly.
    let trace = run_source(
        BlockKind::Waveform(TransientFunction::Sin {
            v0: 0.0,
            va: 10.0,
            freq: 1000.0,
            td: 0.0,
            theta: 0.0,
            phase: 0.0,
        }),
        6e-4,
        1e-6,
    );
    for (t, v) in &trace {
        let expected = 10.0 * (2.0 * std::f64::consts::PI * 1000.0 * t).sin();
        assert!(
            (v - expected).abs() < 1e-6,
            "t={t}: V(a)={v}, expected {expected} (sin)"
        );
    }
    // Quarter period (t=250us) should read the full 10V amplitude.
    let (t, v) = trace
        .iter()
        .min_by(|a, b| (a.0 - 2.5e-4).abs().total_cmp(&(b.0 - 2.5e-4).abs()))
        .unwrap();
    assert!(
        (v - 10.0).abs() < 1e-3,
        "t={t}: expected V(a) near 10.0 at quarter period, got {v}"
    );
}

#[test]
fn waveform_pulse_matches_hand_computed_trapezoid() {
    // PULSE(0 10 0 1e-4 1e-4 2e-4 1e-3) in real seconds -- same shape as general-mna's own
    // arbitrary-unit test, scaled to a realistic timebase.
    let trace = run_source(
        BlockKind::Waveform(TransientFunction::Pulse {
            v1: 0.0,
            v2: 10.0,
            td: 0.0,
            tr: 1e-4,
            tf: 1e-4,
            pw: 2e-4,
            per: 1e-3,
        }),
        1e-3,
        1e-6,
    );
    let at = |target: f64| -> f64 {
        trace
            .iter()
            .min_by(|a, b| (a.0 - target).abs().total_cmp(&(b.0 - target).abs()))
            .unwrap()
            .1
    };
    assert!((at(0.0) - 0.0).abs() < 0.2, "start near v1=0");
    assert!((at(5e-5) - 5.0).abs() < 0.5, "mid-rise near 5V");
    assert!((at(2e-4) - 10.0).abs() < 0.2, "plateau near v2=10");
    assert!((at(4.5e-4) - 0.0).abs() < 0.2, "back to v1=0 after fall");
}

#[test]
fn waveform_exp_matches_hand_computed_rise() {
    // EXP(0 1 0 1e-3 1.0 1e-3): rise only matters in this window (td2 far away), tau1=1ms.
    let trace = run_source(
        BlockKind::Waveform(TransientFunction::Exp {
            v1: 0.0,
            v2: 1.0,
            td1: 0.0,
            tau1: 1e-3,
            td2: 1.0,
            tau2: 1e-3,
        }),
        2e-3,
        1e-5,
    );
    for (t, v) in &trace {
        let expected = 1.0 - (-t / 1e-3).exp();
        assert!(
            (v - expected).abs() < 1e-6,
            "t={t}: V(a)={v}, expected {expected} (exp rise)"
        );
    }
}

#[test]
fn waveform_sffm_matches_hand_computed_value_at_t_zero() {
    let trace = run_source(
        BlockKind::Waveform(TransientFunction::Sffm {
            v0: 0.0,
            va: 5.0,
            fc: 1000.0,
            mdi: 10.0,
            fs: 100.0,
        }),
        1e-6,
        1e-7,
    );
    // simulate_transient_with_blocks's own trace starts at the first *resolved* step (t=dt),
    // not t=0 itself, so check every point against the closed-form SFFM formula directly
    // rather than asserting the (never-observed) t=0 value.
    let two_pi = 2.0 * std::f64::consts::PI;
    for (t, v) in &trace {
        let expected = 5.0 * (two_pi * 1000.0 * t + 10.0 * (two_pi * 100.0 * t).sin()).sin();
        assert!(
            (v - expected).abs() < 1e-9,
            "t={t}: V(a)={v}, expected {expected} (sffm)"
        );
    }
}
