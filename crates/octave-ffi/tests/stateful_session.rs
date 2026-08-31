//! Integration tests for [`OctaveSession`]'s own *stateful* API (`call_start`/`call_stateful`/
//! `call_readonly_stateful`/`rk4_step_xc_stateful`), run against a real `octave-cli` -- the
//! `kind=octblock` counterpart to `tests/session.rs`'s own `kind=octfunc` coverage. See the
//! crate's own module doc comment, "Stateful blocks (`kind=octblock`)", and
//! `general-simulator`'s own `book/dev-guide/src/octave-blocks.md` for the protocol these
//! exercise.

use octave_ffi::{OctaveError, OctaveSession};

fn fixtures_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/stateful")
}

fn spawn() -> OctaveSession {
    let mut session = OctaveSession::spawn().expect("octave-cli must be on PATH for this test");
    session.add_path(&fixtures_dir()).unwrap();
    session
}

/// State genuinely carries forward across several calls to the same instance -- an
/// `accumulator` block summing its own input every call.
#[test]
fn state_accumulates_correctly_across_multiple_steps() {
    let mut session = spawn();
    session.call_start("accumulator", "A").unwrap();

    let mut running = 0.0;
    for u in [3.0, 5.0, -2.0, 10.0] {
        running += u;
        let out = session
            .call_stateful("accumulator", "A", &[0.0, 0.0, u], None, 1)
            .unwrap();
        assert_eq!(out, vec![running]);
    }
}

/// Two independent instances of the *same* function, interleaved call by call, never
/// contaminate each other's own state -- the core per-instance-state-in-one-shared-global-struct
/// guarantee this protocol exists to provide.
#[test]
fn two_instances_of_same_stateful_function_do_not_contaminate_each_others_state() {
    let mut session = spawn();
    session.call_start("accumulator", "A").unwrap();
    session.call_start("accumulator", "B").unwrap();

    let mut running_a = 0.0;
    let mut running_b = 0.0;
    for i in 1..=6 {
        let ua = i as f64;
        let ub = -(i as f64) * 2.0;
        running_a += ua;
        running_b += ub;
        let out_a = session
            .call_stateful("accumulator", "A", &[0.0, 0.0, ua], None, 1)
            .unwrap();
        let out_b = session
            .call_stateful("accumulator", "B", &[0.0, 0.0, ub], None, 1)
            .unwrap();
        assert_eq!(out_a, vec![running_a], "instance A contaminated at i={i}");
        assert_eq!(out_b, vec![running_b], "instance B contaminated at i={i}");
    }

    // Interleave in the opposite order too, to rule out an ordering-dependent bug.
    for i in 1..=6 {
        let ub = -(i as f64);
        let ua = (i as f64) * 3.0;
        running_b += ub;
        running_a += ua;
        let out_b = session
            .call_stateful("accumulator", "B", &[0.0, 0.0, ub], None, 1)
            .unwrap();
        let out_a = session
            .call_stateful("accumulator", "A", &[0.0, 0.0, ua], None, 1)
            .unwrap();
        assert_eq!(
            out_b,
            vec![running_b],
            "instance B contaminated (reversed) at i={i}"
        );
        assert_eq!(
            out_a,
            vec![running_a],
            "instance A contaminated (reversed) at i={i}"
        );
    }
}

/// The `xc` continuous-state vector round-trips correctly through
/// [`OctaveSession::rk4_step_xc_stateful`]/[`OctaveSession::call_stateful`] -- a first-order
/// charge, `xc_dot = 1 - xc`, checked against its own analytic solution `xc(t) = 1 - e^{-t}`
/// (the same precedent `doc-verify/pyblock/xc_example.cir`'s own `xc_decay.py` established).
#[test]
fn xc_vector_round_trips_and_matches_analytic_solution() {
    let mut session = spawn();
    session.call_start("xc_charge", "C1").unwrap();

    let dt = 0.001;
    let steps = 2000; // t = 2.0
    let mut xc = vec![0.0_f64];
    let mut t = 0.0;
    for _ in 0..steps {
        xc = session
            .rk4_step_xc_stateful("xc_charge_derivative", "C1", &xc, &[0.0], dt)
            .unwrap();
        t += dt;
        let out = session
            .call_stateful("xc_charge_output_xc", "C1", &[t, dt, 0.0], Some(&xc), 1)
            .unwrap();
        assert_eq!(out, vec![xc[0]]);
    }

    let analytic = 1.0 - (-t).exp();
    assert!(
        (xc[0] - analytic).abs() < 1e-6,
        "xc={} analytic={} diff={}",
        xc[0],
        analytic,
        (xc[0] - analytic).abs()
    );
}

/// `<function>_update.m`, when present, is the dedicated place discrete bookkeeping commits
/// forward -- `output`'s own returned `new_state` here deliberately does *not* advance the
/// instance's state (see `counter.m`'s own comment), so state only moves when `_update` is
/// separately called, confirmed by checking the output sequence directly.
#[test]
fn update_present_commits_state_forward_separately_from_output() {
    let mut session = spawn();
    session.call_start("counter", "K").unwrap();

    // Before any update call, state stays 0 forever -- output always reports `0 + u`.
    for u in [1.0, 2.0, 3.0] {
        let out = session
            .call_stateful("counter", "K", &[0.0, 0.0, u], None, 1)
            .unwrap();
        assert_eq!(out, vec![u], "output should read state=0 before any update");
    }

    // Now call _update.m explicitly (as dae-runtime's own per-step dispatch would, once per
    // resolved step) -- state should advance by 1 each time, and output should reflect it.
    for expected_state in 1..=3 {
        session
            .call_stateful("counter_update", "K", &[0.0, 0.0, 0.0], None, 0)
            .unwrap();
        let out = session
            .call_stateful("counter", "K", &[0.0, 0.0, 10.0], None, 1)
            .unwrap();
        assert_eq!(out, vec![expected_state as f64 + 10.0]);
    }
}

/// `<function>_update.m` absent: `accumulator` has no `_update.m` fixture at all, so its own
/// `output`'s returned `new_state` is the only state advance -- confirming the "if absent" half
/// of the update contract, not just the "if present" half above.
#[test]
fn update_absent_output_is_the_only_state_advance() {
    let mut session = spawn();
    session.call_start("accumulator", "N").unwrap();
    assert_eq!(
        session
            .call_stateful("accumulator", "N", &[0.0, 0.0, 7.0], None, 1)
            .unwrap(),
        vec![7.0]
    );
    assert_eq!(
        session
            .call_stateful("accumulator", "N", &[0.0, 0.0, 8.0], None, 1)
            .unwrap(),
        vec![15.0]
    );
}

/// A runtime error mid-sequence (calling `erroring` with a negative input, which its own `.m`
/// file explicitly rejects) is caught cleanly as `OctaveError::Runtime`, and -- the specific
/// guarantee this protocol depends on -- **the instance's own state slot is left exactly as it
/// was before the failed call**, not silently overwritten with garbage, because Octave never
/// performs the left-hand-side state assignment when the right-hand side raises.
#[test]
fn failed_stateful_call_does_not_corrupt_instance_state() {
    let mut session = spawn();
    session.call_start("erroring", "E").unwrap();

    let out1 = session
        .call_stateful("erroring", "E", &[0.0, 0.0, 5.0], None, 1)
        .unwrap();
    assert_eq!(out1, vec![5.0]);

    // This call raises inside erroring.m -- state must remain at 5.0 afterward, not become
    // corrupted (e.g. left partially assigned, or clobbered with a garbage/zero value).
    let err = session
        .call_stateful("erroring", "E", &[0.0, 0.0, -1.0], None, 1)
        .unwrap_err();
    assert!(matches!(err, OctaveError::Runtime { .. }));

    // The very next call proves the state slot is still exactly 5.0 -- if it had been
    // corrupted (e.g. reset to 0), this would return something other than 5.0 + 3.0 = 8.0.
    let out2 = session
        .call_stateful("erroring", "E", &[0.0, 0.0, 3.0], None, 1)
        .unwrap();
    assert_eq!(
        out2,
        vec![8.0],
        "instance state was corrupted by the failed call"
    );

    // And a second independent error-then-recovery cycle, ruling out a lucky ordering.
    let err2 = session
        .call_stateful("erroring", "E", &[0.0, 0.0, -100.0], None, 1)
        .unwrap_err();
    assert!(matches!(err2, OctaveError::Runtime { .. }));
    let out3 = session
        .call_stateful("erroring", "E", &[0.0, 0.0, 2.0], None, 1)
        .unwrap();
    assert_eq!(out3, vec![10.0]);
}
