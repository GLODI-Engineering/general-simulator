//! A parameterised `.subckt` solves, with each instance's own values (general-mna#63).
//!
//! Until `general-mna 0.11.0` this binary could not run a parameterised subcircuit at all.
//! Both halves of the grammar parsed — the definition's `Rv=1000` and the call site's
//! `Rv=100` — and nothing bound them, so the body's `{Rv}` arrived at the builder as the name
//! of a parameter nothing declared and the run stopped with
//! `Evaluate(MissingSymbol): ... missing value for symbol 'Rv'`.
//!
//! The reason it is worth a test *here*, in the repository that ships the binary, is that the
//! repair lives in a path dependency: nothing in these crates changed, so without this file the
//! release that carries the capability would have no assertion that the capability is there. And
//! the failure mode to guard against is not a crash. A binding pass that silently fell back to
//! the declared default would still produce a complete, plausible run — of a circuit nobody
//! wrote. So the numbers below are derived from the deck rather than recorded from a run.

use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Runs one deck at its operating point and returns `(stdout, stderr, success)`.
fn run(deck: &str) -> (String, String, bool) {
    let output = Command::new(env!("CARGO_BIN_EXE_general-simulator"))
        .arg(fixture(deck))
        .args(["--dt", "1e-6"])
        .output()
        .expect("failed to run the general-simulator binary");
    (
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
        output.status.success(),
    )
}

#[test]
fn each_instance_of_one_definition_solves_with_the_value_it_was_given() {
    let (stdout, stderr, ok) = run("rpart_two_instances.cir");
    assert!(ok, "the run failed\nstdout: {stdout}\nstderr: {stderr}");

    let mut lines = stdout.lines();
    assert_eq!(
        lines.next(),
        Some("t,V(A),V(MID),I(V1)"),
        "unexpected header in\n{stdout}"
    );
    let row: Vec<f64> = lines
        .next()
        .unwrap_or_else(|| panic!("no data row in\n{stdout}"))
        .split(',')
        .map(|field| {
            field
                .parse::<f64>()
                .unwrap_or_else(|e| panic!("{field:?} is not a number: {e}"))
        })
        .collect();

    // 5 V across 100 Ω + 220 Ω: the current is 5/320 and the mid-node divides as 220/320. The
    // source branch current is reported into the source, hence the sign.
    assert_eq!(row[0], 0.0, "{stdout}");
    assert!(
        (row[1] - 5.0).abs() < 1e-12,
        "V(A) = {} in\n{stdout}",
        row[1]
    );
    assert!(
        (row[2] - 5.0 * 220.0 / 320.0).abs() < 1e-12,
        "V(MID) = {} rather than 5*220/320 = {} in\n{stdout}",
        row[2],
        5.0 * 220.0 / 320.0
    );
    assert!(
        (row[3] + 5.0 / 320.0).abs() < 1e-12,
        "I(V1) = {} rather than -5/320 = {} in\n{stdout}",
        row[3],
        -5.0 / 320.0
    );

    // The assertion that fails if the instance values were dropped and the declared default of
    // 1000 Ω used for both: that circuit draws 2.5 mA and divides its mid-node at half.
    assert!(
        (row[3] + 5.0 / 2000.0).abs() > 1e-6,
        "both instances took the definition's default instead of their own value:\n{stdout}"
    );
    assert!((row[2] - 2.5).abs() > 1e-6, "{stdout}");
}

#[test]
fn a_parameter_the_definition_does_not_declare_stops_the_run_and_names_it() {
    // A mistyped parameter name is how a deck comes to describe one circuit and solve another.
    // Ignoring the override would run 1000 Ω where the deck says 100 and report the answer as
    // if it were the right one, so the run stops -- and the message has to be enough to find
    // the line, which means naming the instance as well as the parameter.
    let (stdout, stderr, ok) = run("rpart_mistyped_parameter.cir");
    assert!(!ok, "the mistyped deck ran anyway:\n{stdout}");
    let said = format!("{stderr}{stdout}");
    assert!(said.contains("X1"), "does not name the instance: {said}");
    assert!(said.contains("Rw"), "does not name the parameter: {said}");
    assert!(
        said.contains("RPART"),
        "does not name the definition: {said}"
    );
    assert!(
        said.contains("Rv"),
        "does not say what it could have meant: {said}"
    );
}
