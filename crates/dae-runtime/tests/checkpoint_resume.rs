//! Checkpoint/resume, end to end: the defining property is that a run stopped at $T$ and
//! resumed to $2T$ produces a trace **bit-identical** (`==` on every `f64`, not "close") to a
//! single uninterrupted run to $2T$. Every test here is that comparison, on a deck exercising
//! everything the loop carries: a PWM-gated ideal switch and a freewheeling ideal diode
//! (segments, gate states, the backward-Euler fallback and its ringing cooldown), a `prev:`
//! signal, a PI loop, and every stateful built-in block kind.

use std::collections::BTreeMap;

use dae_runtime::checkpoint::Checkpoint;
use dae_runtime::{
    simulate_transient_with_blocks_checkpointed, AdaptiveConfig, CheckpointControl, DaeError,
    OperatingPoint, SignalValue, TimeStep,
};
use general_mna::{build_system, System};
use general_spice_core::Dialect;

/// A PWM-driven buck with a real switching loop: the duty is a PI (`kind=pid`) on the measured
/// output, low-pass filtered, with a `prev:`-referenced accumulator, a discrete PI running at
/// its own sample rate, and one of each remaining stateful kind hanging off the same signals.
const DECK: &str = "\
VIN vin 0 12
DSW vin sw idealswitchmodel
DSW kind=ideal_switch r_on=0.05 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.5 g_on=5 \
 gate=block ctrl=GATE
D1 0 sw dmodel
D1 kind=ideal_diode g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.4 g_on=5
L1 sw out 22e-6
C1 out 0 47e-6 ic=3
RL out 0 4
VOUT kind=phys2sig node=out
REF kind=const value=5
ERR kind=sum inputs=REF,VOUT signs=1,-1
PI kind=pid in=ERR kp=0.02 ki=50 kd=0 n=100 clamp_lo=0.05 clamp_hi=0.95 ic=0.4
DUTY kind=tf in=PI num=[1] den=[2e-6,1] y0=0.4
PWM1 kind=pwm freq=100000 in=DUTY
GATE kind=sig2phys domain=voltage in=PWM1
ACC kind=sum inputs=prev:ACC,ERR signs=1,1
DPI kind=discretepid in=ERR kp=0.01 ki=20 kd=0 n=1 ts=5e-6 clamp_lo=-1 clamp_hi=1 \
 integration_method=trapezoidal
DSS kind=discretestatespace in=ERR a=[[0.9]] b=[0.1] c=[1] ts=5e-6
DTF kind=discretetf in=ERR num=[0.5] den=[1,-0.5] ts=5e-6
SS kind=statespace in=ERR a=[[0,1],[-1e10,-2e5]] b=[0,1e10] c=[1,0]
OSC kind=vco in=REF f_min=1 f_max=1e6 ic=0.3
CMP kind=hysteresis in=VOUT high=5.2 low=4.8
LAT kind=srlatch set=CMP reset=PWM1
FF kind=dff clk=PWM1 d=CMP
CNT kind=counter clk=PWM1 modulus=7
PS kind=pspwm f_min=1 f_max=1e6 inputs=REF,REF,DUTY ic=0.1
";

type Row = (
    f64,
    Vec<f64>,
    Vec<(f64, f64)>,
    Vec<f64>,
    BTreeMap<String, SignalValue>,
);

fn row(t: f64, p: OperatingPoint, o: BTreeMap<String, SignalValue>) -> Row {
    (t, p.x, p.diode_z, p.diode_raw_ioff, o)
}

/// Runs `DECK` from `resume` to `t_final`, collecting every row and every mid-run checkpoint
/// `every` asks for, and returns the final checkpoint too.
fn run(
    step: TimeStep,
    t_final: f64,
    resume: Option<&Checkpoint>,
    every: Option<f64>,
) -> Result<(Vec<Row>, Vec<Checkpoint>, Checkpoint), DaeError> {
    let System {
        ideal_diodes,
        ideal_switches,
        gates,
        blocks,
        shared_r_on,
        ..
    } = build_system(DECK, Dialect::Ngspice).unwrap();
    let mut rows = Vec::new();
    let mid = std::cell::RefCell::new(Vec::new());
    let mut sink = |c: &Checkpoint| {
        mid.borrow_mut().push(c.clone());
        Ok(())
    };
    let last = simulate_transient_with_blocks_checkpointed(
        DECK,
        Dialect::Ngspice,
        &ideal_diodes,
        &ideal_switches,
        &blocks,
        &gates,
        shared_r_on,
        None,
        t_final,
        step,
        1_000_000,
        CheckpointControl {
            resume,
            every,
            on_checkpoint: every.map(|_| &mut sink as &mut dae_runtime::CheckpointSink<'_>),
            final_snapshot: true,
        },
        |t, p, o| {
            rows.push(row(t, p, o));
            Ok(())
        },
    )?;
    Ok((
        rows,
        mid.into_inner(),
        last.expect("final_snapshot was requested"),
    ))
}

fn assert_bit_identical(whole: &[Row], first: &[Row], second: &[Row]) {
    assert_eq!(whole.len(), first.len() + second.len(), "row counts differ");
    for (k, (w, s)) in whole.iter().zip(first.iter().chain(second)).enumerate() {
        assert!(
            w.0.to_bits() == s.0.to_bits(),
            "row {k}: t differs ({} vs {})",
            w.0,
            s.0
        );
        assert_eq!(w.1.len(), s.1.len());
        for (i, (a, b)) in w.1.iter().zip(&s.1).enumerate() {
            assert!(
                a.to_bits() == b.to_bits(),
                "row {k} (t={}), unknown {i}: {a:e} vs {b:e}",
                w.0
            );
        }
        assert_eq!(w.2, s.2, "row {k}: diode segments differ");
        assert_eq!(w.3, s.3, "row {k}: diode Norton currents differ");
        assert_eq!(w.4, s.4, "row {k}: block outputs differ");
    }
}

/// Fixed stepping, resumed from the *final* checkpoint of the first half: the uninterrupted
/// run steps through exactly `t_half` too, so the two must agree to the bit.
fn resume_from_final_is_bit_identical(step: TimeStep) {
    let t_half = 1.5e-4;
    let (whole, _, _) = run(step, 2.0 * t_half, None, None).unwrap();
    let (first, _, ckpt) = run(step, t_half, None, None).unwrap();
    // Through bytes, not just the in-memory struct: the file format is part of the contract.
    let ckpt = Checkpoint::from_bytes(&ckpt.to_bytes().unwrap()).unwrap();
    let (second, _, _) = run(step, 2.0 * t_half, Some(&ckpt), None).unwrap();
    assert!(!second.is_empty(), "the resumed half produced no rows");
    assert_bit_identical(&whole, &first, &second);

    // The deck genuinely switches and rings -- otherwise this test would prove little.
    let first_segments = &whole[0].2;
    assert!(
        whole.iter().any(|r| &r.2 != first_segments),
        "the diode never changed segment"
    );
    let gate_on = whole
        .iter()
        .filter(|r| r.4["PWM1"].as_scalar() == Some(1.0))
        .count();
    assert!(
        gate_on > 0 && gate_on < whole.len(),
        "the switch never toggled"
    );
}

#[test]
fn fixed_step_resume_is_bit_identical_to_an_uninterrupted_run() {
    resume_from_final_is_bit_identical(TimeStep::Fixed(2e-7));
}

/// A checkpoint handed out *mid-run* by `every` (the crash-recovery case) is just as good as
/// the final one: resuming from it reproduces the rest of the run exactly.
///
/// For adaptive stepping this is also the *only* form in which bit-identity against a single
/// uninterrupted run can hold: a run's final step is clamped to land exactly on `t_final`,
/// a point the uninterrupted run never stepped to, so resuming from a final adaptive
/// checkpoint is exact with respect to its own state but follows a different (equally valid)
/// step sequence from there. A periodic checkpoint is taken *after* an ordinary accepted step
/// and changes nothing about the sequence.
fn periodic_mid_run_checkpoint_resumes_bit_identically(step: TimeStep) {
    let (whole, mid, _) = run(step, 3e-4, None, Some(1e-4)).unwrap();
    assert!(
        mid.len() >= 2,
        "expected at least two periodic checkpoints, got {}",
        mid.len()
    );
    let ckpt = &mid[1];
    let before = whole.iter().take_while(|r| r.0 <= ckpt.t).count();
    assert!(before > 0 && before < whole.len());
    let (second, _, _) = run(step, 3e-4, Some(ckpt), None).unwrap();
    assert_bit_identical(&whole, &whole[..before], &second);
}

#[test]
fn a_periodic_mid_run_checkpoint_resumes_bit_identically_fixed_step() {
    periodic_mid_run_checkpoint_resumes_bit_identically(TimeStep::Fixed(2e-7));
}

#[test]
fn a_periodic_mid_run_checkpoint_resumes_bit_identically_adaptive() {
    periodic_mid_run_checkpoint_resumes_bit_identically(TimeStep::Adaptive(AdaptiveConfig {
        dt_init: 1e-7,
        dt_min: 1e-10,
        dt_max: 1e-6,
        reltol: 1e-4,
        abstol: 1e-9,
    }));
}

/// Resuming from an adaptive run's *final* checkpoint: exact continuation of the state (the
/// resumed run's first row is one controller step past the checkpoint, not a restart from
/// `dt_init` with a forced backward-Euler step), even though the step sequence legitimately
/// differs from an uninterrupted run's -- see the note above.
#[test]
fn adaptive_resume_from_final_continues_the_controller_not_a_restart() {
    let step = TimeStep::Adaptive(AdaptiveConfig {
        dt_init: 1e-7,
        dt_min: 1e-10,
        dt_max: 1e-6,
        reltol: 1e-4,
        abstol: 1e-9,
    });
    let (_, _, ckpt) = run(step, 1.5e-4, None, None).unwrap();
    let dt_next = ckpt
        .dt_next
        .expect("an adaptive checkpoint carries dt_next");
    assert!(ckpt.step_index > 0 && ckpt.accepted_steps == ckpt.step_index);
    let (second, _, _) = run(step, 3e-4, Some(&ckpt), None).unwrap();
    let first_dt = second[0].0 - ckpt.t;
    // The first resumed step is the controller's own suggestion (possibly clamped to a predicted
    // gate edge), never `dt_init` -- which is 10x smaller than dt_max and would show. The
    // tolerance is the rounding of `t + dt` itself, not a numerical-agreement fudge.
    assert!(
        first_dt <= dt_next * (1.0 + 1e-6),
        "first resumed dt {first_dt:e} exceeds the saved dt_next {dt_next:e}"
    );
    assert!(second.last().unwrap().0 <= 3e-4 * (1.0 + 1e-12));
}

/// Loading into an edited deck is refused, naming the unknowns on each side.
#[test]
fn resuming_into_a_different_deck_is_refused() {
    let (_, _, ckpt) = run(TimeStep::Fixed(2e-7), 1e-5, None, None).unwrap();
    let edited = DECK.replace("RL out 0 4", "RL out 0 5");
    let System {
        ideal_diodes,
        ideal_switches,
        gates,
        blocks,
        shared_r_on,
        ..
    } = build_system(&edited, Dialect::Ngspice).unwrap();
    let err = simulate_transient_with_blocks_checkpointed(
        &edited,
        Dialect::Ngspice,
        &ideal_diodes,
        &ideal_switches,
        &blocks,
        &gates,
        shared_r_on,
        None,
        2e-5,
        TimeStep::Fixed(2e-7),
        1_000_000,
        CheckpointControl {
            resume: Some(&ckpt),
            ..CheckpointControl::default()
        },
        |_, _, _| Ok(()),
    )
    .unwrap_err();
    assert!(
        matches!(err, DaeError::CheckpointDeckMismatch { .. }),
        "expected a deck mismatch, got {err:?}"
    );
}

/// A resumed run whose `t_final` is already behind the checkpoint does nothing, rather than
/// stepping backwards or panicking.
#[test]
fn resuming_past_t_final_is_an_empty_run() {
    let (_, _, ckpt) = run(TimeStep::Fixed(2e-7), 1e-5, None, None).unwrap();
    let (rows, _, last) = run(TimeStep::Fixed(2e-7), 1e-5, Some(&ckpt), None).unwrap();
    assert!(rows.is_empty());
    assert_eq!(last, ckpt);

    // And a run that never asked for a snapshot gets none, which is what lets a deck with a
    // `cscript` block keep running unchanged.
    let System {
        ideal_diodes,
        ideal_switches,
        gates,
        blocks,
        shared_r_on,
        ..
    } = build_system(DECK, Dialect::Ngspice).unwrap();
    let none = simulate_transient_with_blocks_checkpointed(
        DECK,
        Dialect::Ngspice,
        &ideal_diodes,
        &ideal_switches,
        &blocks,
        &gates,
        shared_r_on,
        None,
        1e-6,
        TimeStep::Fixed(2e-7),
        1_000_000,
        CheckpointControl::default(),
        |_, _, _| Ok(()),
    )
    .unwrap();
    assert!(none.is_none());
}

/// A `kind=pyblock`'s own state object rides along as `pickle` bytes: a split run through the
/// hand-written PI controller fixture (a `dict` holding the integral) agrees bit for bit.
#[cfg(feature = "python")]
#[test]
fn a_pyblock_state_round_trips_through_pickle() {
    let py = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/pi_controller.py");
    let deck = format!(
        "V1 a 0 5\nR1 a 0 1k\nREF kind=const value=1\nVA kind=phys2sig node=a\n\
         ERR kind=sum inputs=REF,VA signs=1,-1\nPI kind=pyblock path=\"{}\" in=ERR\n\
         F kind=tf in=PI num=[1] den=[1e-3,1]\n",
        py.display()
    );
    let run = |t_final: f64, resume: Option<&Checkpoint>| {
        let System {
            ideal_diodes,
            ideal_switches,
            gates,
            blocks,
            shared_r_on,
            ..
        } = build_system(&deck, Dialect::Ngspice).unwrap();
        let mut rows = Vec::new();
        let last = simulate_transient_with_blocks_checkpointed(
            &deck,
            Dialect::Ngspice,
            &ideal_diodes,
            &ideal_switches,
            &blocks,
            &gates,
            shared_r_on,
            None,
            t_final,
            TimeStep::Fixed(1e-4),
            1_000_000,
            CheckpointControl {
                resume,
                final_snapshot: true,
                ..CheckpointControl::default()
            },
            |t, p, o| {
                rows.push(row(t, p, o));
                Ok(())
            },
        )
        .unwrap()
        .unwrap();
        (rows, last)
    };
    let (whole, _) = run(2e-2, None);
    let (first, ckpt) = run(1e-2, None);
    let ckpt = Checkpoint::from_bytes(&ckpt.to_bytes().unwrap()).unwrap();
    let (second, _) = run(2e-2, Some(&ckpt));
    assert_bit_identical(&whole, &first, &second);
    // The integral is genuinely nonzero by the split, i.e. there was state to carry.
    assert_ne!(first.last().unwrap().4["PI"], SignalValue::Scalar(0.0));
}
