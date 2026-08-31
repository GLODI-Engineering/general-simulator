//! Integration tests run against a real `octave-cli` (11.1.0 at the time this was written) --
//! see the crate's own module doc comment for the protocol these exercise. Every test here
//! spawns its own [`OctaveSession`] (the whole point of a persistent session is one per *run*,
//! not one per test, but tests need isolation from each other more than they need to share
//! Octave's own startup cost).

use std::path::Path;

use octave_ffi::{OctaveError, OctaveSession};

fn fixtures_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// A single call, with a single scalar output, returns the expected value -- the basic
/// happy path every other test builds on.
#[test]
fn single_call_returns_expected_value() {
    let mut session = OctaveSession::spawn().expect("octave-cli must be on PATH for this test");
    session.add_path(&fixtures_dir()).unwrap();
    let out = session.call("add_one", &[3.0], 1).unwrap();
    assert_eq!(out, vec![4.0]);
}

/// Multiple outputs come back in declared order, matching Octave's own `[a, b] = f(...)`
/// multi-return convention.
#[test]
fn multi_output_call_returns_values_in_order() {
    let mut session = OctaveSession::spawn().expect("octave-cli must be on PATH for this test");
    session.add_path(&fixtures_dir()).unwrap();
    let out = session.call("two_out", &[10.0, 3.0], 2).unwrap();
    assert_eq!(out, vec![13.0, 7.0]);
}

/// `%.17g` on the Octave side plus `f64::parse` on the Rust side round-trips a genuine
/// fractional double bit-for-bit, not just a round number -- 0.1 + 0.2 famously has a
/// precision tail (0.30000000000000004) that a lossy round-trip would corrupt.
#[test]
fn fractional_double_round_trips_bit_for_bit() {
    let mut session = OctaveSession::spawn().expect("octave-cli must be on PATH for this test");
    session.add_path(&fixtures_dir()).unwrap();
    let x: f64 = 0.1 + 0.2;
    assert_eq!(x.to_bits(), 0.30000000000000004_f64.to_bits());
    let out = session.call("identity", &[x], 1).unwrap();
    assert_eq!(out.len(), 1);
    assert_eq!(
        out[0].to_bits(),
        x.to_bits(),
        "expected bit-for-bit round trip, got {} vs {x}",
        out[0]
    );
}

/// A runtime error (wrong arity: `add_one` requires one argument, called with zero) is caught
/// cleanly by the crate's own `try`/`catch` wrapper and reported as `OctaveError::Runtime`, not
/// a panic and not misparsed as a numeric output -- **and the session survives, remaining fully
/// usable for subsequent calls.** This is the error-then-recovery sequence called out
/// specifically in the design: no respawn should be necessary.
#[test]
fn runtime_error_is_caught_and_session_survives_for_later_calls() {
    let mut session = OctaveSession::spawn().expect("octave-cli must be on PATH for this test");
    session.add_path(&fixtures_dir()).unwrap();

    let err = session.call("add_one", &[], 1).unwrap_err();
    match err {
        OctaveError::Runtime { message } => {
            assert!(!message.is_empty(), "expected a non-empty Octave message");
        }
        other => panic!("expected OctaveError::Runtime, got {other:?}"),
    }

    // The same session, immediately after a caught error, must still work correctly.
    let out = session.call("add_one", &[41.0], 1).unwrap();
    assert_eq!(out, vec![42.0]);

    // And a second, independent error-then-recovery cycle, to make sure the first recovery
    // wasn't a fluke of some lucky ordering.
    let err2 = session.call("add_one", &[1.0, 2.0, 3.0], 1).unwrap_err();
    assert!(matches!(err2, OctaveError::Runtime { .. }));
    let out2 = session.call("add_one", &[99.0], 1).unwrap();
    assert_eq!(out2, vec![100.0]);
}

/// If the persistent process dies mid-run (here: killed outright, simulating a crash), a
/// blocking read on its stdout must not hang the caller forever -- it must detect EOF/broken
/// pipe and return a clear `OctaveError::ProcessExited`. This actually kills the child process
/// and confirms the call that follows returns promptly with the right error, not by reviewing
/// the code.
#[test]
fn process_dies_mid_run_reports_clean_error_instead_of_hanging() {
    let mut session = OctaveSession::spawn().expect("octave-cli must be on PATH for this test");
    session.add_path(&fixtures_dir()).unwrap();

    // Confirm the session works before killing it, so a failure below is unambiguously about
    // the kill, not some pre-existing setup problem.
    assert_eq!(session.call("add_one", &[1.0], 1).unwrap(), vec![2.0]);

    // Reach into the child process directly and kill it -- simulating a crash or an external
    // `kill -9`, not a graceful shutdown this crate's own Drop impl would otherwise perform.
    session.kill_for_test();

    // This call's write may or may not itself fail (killing a process doesn't synchronously
    // close its stdin from this side), but the subsequent read must observe EOF, not hang.
    // Bound this with a background thread + timeout so a genuine hang fails the test instead of
    // blocking the whole suite forever.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = session.call("add_one", &[1.0], 1);
        let _ = tx.send(result);
    });
    let result = rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("call after killing the process hung instead of returning an error promptly");
    match result {
        Err(OctaveError::ProcessExited { .. }) => {}
        other => panic!("expected OctaveError::ProcessExited, got {other:?}"),
    }
}

/// `octave-cli` not existing on `PATH` at all is reported as `OctaveError::NotFound`, not a
/// panic -- exercised by spawning a process under a `PATH` that genuinely does not contain
/// `octave-cli`, the same technique used to test "tool not installed" paths elsewhere in this
/// workspace.
#[test]
fn missing_octave_cli_reports_not_found() {
    let result =
        OctaveSession::spawn_with_command(Path::new("octave-cli-that-does-not-exist-anywhere"));
    match result {
        Err(OctaveError::NotFound { .. }) => {}
        other => panic!("expected OctaveError::NotFound, got {other:?}"),
    }
}

/// Two block instances calling two genuinely different `.m` functions (in two different
/// directories, each needing its own `add_path`), interleaved call by call on **one shared
/// session**, never contaminate each other -- the actual multi-instance-sharing-one-session
/// guarantee this crate exists to provide. If anything about call framing leaked between calls
/// (a stray marker, a workspace variable collision), this would show up as a wrong value or an
/// outright desync error.
#[test]
fn two_instances_of_different_functions_share_one_session_without_contamination() {
    let mut session = OctaveSession::spawn().expect("octave-cli must be on PATH for this test");
    session.add_path(&fixtures_dir().join("dir_a")).unwrap();
    session.add_path(&fixtures_dir().join("dir_b")).unwrap();

    for i in 1..=5 {
        let x = i as f64;
        let gain_out = session.call("gain", &[x], 1).unwrap();
        let square_out = session.call("square", &[x], 1).unwrap();
        assert_eq!(gain_out, vec![x * 2.0], "gain contaminated at i={i}");
        assert_eq!(square_out, vec![x * x], "square contaminated at i={i}");
    }

    // Interleave in the opposite order too, to rule out an ordering-dependent bug.
    for i in 1..=5 {
        let x = i as f64;
        let square_out = session.call("square", &[x], 1).unwrap();
        let gain_out = session.call("gain", &[x], 1).unwrap();
        assert_eq!(
            square_out,
            vec![x * x],
            "square contaminated (reversed) at i={i}"
        );
        assert_eq!(
            gain_out,
            vec![x * 2.0],
            "gain contaminated (reversed) at i={i}"
        );
    }
}
