//! `--checkpoint-out` / `--checkpoint-every` / `--resume`, through the real binary and the CSV
//! writer: a run split in two must reproduce the uninterrupted run's output **byte for byte**
//! (the same property `dae-runtime`'s own `checkpoint_resume.rs` checks on the in-memory trace),
//! a periodic checkpoint left on disk by a longer run resumes correctly, and a checkpoint refuses
//! to load into an edited deck.

use std::path::{Path, PathBuf};
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_general-simulator"))
        .args(args)
        .output()
        .expect("failed to run general-simulator binary")
}

fn stdout_of(args: &[&str]) -> String {
    let output = run(args);
    assert!(
        output.status.success(),
        "general-simulator exited with {:?}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("stdout was not valid UTF-8")
}

fn transient(netlist: &Path, t_final: &str, extra: &[&str]) -> String {
    let mut args = vec![
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        t_final,
        "--dt",
        "2e-7",
    ];
    args.extend_from_slice(extra);
    stdout_of(&args)
}

#[test]
fn a_split_run_reproduces_the_uninterrupted_csv_byte_for_byte() {
    let dir = tempdir("split");
    let ckpt = dir.join("half.ckpt");
    let ckpt = ckpt.to_str().unwrap();
    let netlist = fixture("checkpoint_buck.cir");

    let whole = transient(&netlist, "3e-4", &[]);
    let first = transient(&netlist, "1.5e-4", &["--checkpoint-out", ckpt]);
    let second = transient(&netlist, "3e-4", &["--resume", ckpt]);

    let header = whole.lines().next().unwrap();
    assert_eq!(first.lines().next().unwrap(), header);
    assert_eq!(second.lines().next().unwrap(), header);
    let joined: String =
        first
            .lines()
            .chain(second.lines().skip(1))
            .fold(String::new(), |mut acc, l| {
                acc.push_str(l);
                acc.push('\n');
                acc
            });
    assert_eq!(whole.lines().count(), 1501, "1500 rows plus the header");
    assert!(
        whole == joined,
        "the split run's CSV differs from the uninterrupted one"
    );
}

#[test]
fn a_periodic_checkpoint_left_on_disk_resumes_correctly() {
    let dir = tempdir("periodic");
    let ckpt = dir.join("periodic.ckpt");
    let ckpt = ckpt.to_str().unwrap();
    let netlist = fixture("checkpoint_buck.cir");

    // A --checkpoint-every run also writes its final state, replacing the last periodic one,
    // so the file left on disk is the 2.5e-4 checkpoint; the resumed CSV's first timestamp
    // reveals where it continued from, and from there on it must equal the whole run's tail.
    let whole = transient(&netlist, "3e-4", &[]);
    transient(
        &netlist,
        "2.5e-4",
        &["--checkpoint-out", ckpt, "--checkpoint-every", "1e-4"],
    );
    let second = transient(&netlist, "3e-4", &["--resume", ckpt]);
    let first_t: f64 = second
        .lines()
        .nth(1)
        .unwrap()
        .split(',')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(
        first_t > 2.5e-4,
        "resumed from t={first_t}, expected past 2.5e-4"
    );
    let tail: Vec<&str> = whole
        .lines()
        .skip(1)
        .filter(|l| l.split(',').next().unwrap().parse::<f64>().unwrap() >= first_t)
        .collect();
    let resumed: Vec<&str> = second.lines().skip(1).collect();
    assert_eq!(tail, resumed);
}

#[test]
fn resuming_into_an_edited_deck_is_refused() {
    let dir = tempdir("mismatch");
    let ckpt = dir.join("x.ckpt");
    let ckpt = ckpt.to_str().unwrap();
    let netlist = fixture("checkpoint_buck.cir");
    transient(&netlist, "1e-5", &["--checkpoint-out", ckpt]);

    let edited = dir.join("edited.cir");
    std::fs::write(
        &edited,
        std::fs::read_to_string(&netlist)
            .unwrap()
            .replace("RL out 0 4", "RL out 0 5"),
    )
    .unwrap();
    let output = run(&[
        edited.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "2e-5",
        "--dt",
        "2e-7",
        "--resume",
        ckpt,
    ]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("CheckpointDeckMismatch"),
        "expected a deck-mismatch error, got: {stderr}"
    );
}

#[test]
fn checkpoint_every_without_an_output_path_is_rejected() {
    let netlist = fixture("checkpoint_buck.cir");
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "1e-5",
        "--dt",
        "2e-7",
        "--checkpoint-every",
        "1e-6",
    ]);
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--checkpoint-every needs"));
}

fn tempdir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "general-simulator-checkpoint-{tag}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
