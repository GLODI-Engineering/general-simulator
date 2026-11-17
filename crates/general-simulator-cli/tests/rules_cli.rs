//! `kind=rule` cards through the real binary: what gets logged, what the process returns, and
//! the one workflow the whole feature is built around -- a run that a rule stops, a rule that is
//! then adjusted, and a resume that continues from exactly where it stopped.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("gs-rules-{}", std::process::id()));
    fs::create_dir_all(&dir).expect("scratch dir");
    dir.join(name)
}

fn write(name: &str, contents: &str) -> PathBuf {
    let path = scratch(name);
    fs::write(&path, contents).expect("writing the netlist");
    path
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_general-simulator"))
        .args(args)
        .output()
        .expect("failed to run general-simulator binary")
}

/// A buck whose inductor current overshoots on start-up.
const BUCK: &str = "\
VIN vin 0 12
DSW vin sw idealswitchmodel
DSW kind=ideal_switch r_on=0.05 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.5 g_on=5 gate=block ctrl=GATE
D1 0 sw dmodel
D1 kind=ideal_diode g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.4 g_on=5
L1 sw out 22e-6
C1 out 0 47e-6 ic=3
RL out 0 4
VOUT kind=phys2sig node=out
REF kind=const value=5
ERR kind=sum inputs=REF,VOUT signs=1,-1
PI kind=pid in=ERR kp=0.02 ki=50 kd=0 n=100 clamp_lo=0.05 clamp_hi=0.95 ic=0.4
PWM1 kind=pwm freq=100000 in=PI
GATE kind=sig2phys domain=voltage in=PWM1
";

/// A half-bridge leg whose low-side gate is generated as PWM(1-d) rather than NOT(PWM(d)) --
/// both switches conduct together on every cycle.
const SHOOT_THROUGH_LEG: &str = "\
VIN vin 0 400
DSH vin mid idealswitchmodel
DSH kind=ideal_switch r_on=0.01 g_breakdown=0 v_breakdown=-1000 g_off=0 v_th=0.5 g_on=100 gate=block ctrl=GH
DSL mid 0 idealswitchmodel
DSL kind=ideal_switch r_on=0.01 g_breakdown=0 v_breakdown=-1000 g_off=0 v_th=0.5 g_on=100 gate=block ctrl=GL
L1 mid out 1e-3
R1 out 0 10
DUTY kind=const value=0.4
ONE kind=const value=1
COMP kind=sum inputs=ONE,DUTY signs=1,-1
PWMH kind=pwm freq=20000 in=DUTY
PWML kind=pwm freq=20000 in=COMP
GH kind=sig2phys domain=voltage in=PWMH
GL kind=sig2phys domain=voltage in=PWML
";

#[test]
fn a_violated_rule_logs_edges_and_reports_through_the_exit_code() {
    let netlist = write(
        "edges.cir",
        &format!(
            "{BUCK}\nR_isat kind=rule type=above target=I(L1) limit=3 unit=A severity=error\n"
        ),
    );
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "3e-4",
        "--dt",
        "1e-7",
    ]);
    assert_eq!(output.status.code(), Some(2), "a rule at error severity");

    let events = fs::read_to_string(netlist.with_extension("events.jsonl")).expect("the log");
    let enters = events.matches("\"kind\":\"enter\"").count();
    let leaves = events.matches("\"kind\":\"leave\"").count();
    assert!(
        enters > 0 && enters == leaves,
        "{enters} enters, {leaves} leaves"
    );
    // The point of edge logging: a 3000-step run reports a handful of lines, not thousands.
    assert!(
        enters < 50,
        "{enters} enters is per-sample logging, not per-edge"
    );

    let text = fs::read_to_string(netlist.with_extension("events.txt")).expect("the text log");
    assert!(text.contains("--- rules ---"), "{text}");
}

#[test]
fn two_switches_of_a_leg_conducting_together_stops_the_run() {
    let netlist = write(
        "leg.cir",
        &format!(
            "{SHOOT_THROUGH_LEG}\n\
             R_leg kind=rule type=forbidden_state switches=DSH,DSL severity=fatal action=stop\n"
        ),
    );
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "2e-4",
        "--dt",
        "1e-7",
    ]);
    assert_eq!(output.status.code(), Some(3), "stopped by a rule");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("stopped by rule 'R_leg'"), "{stderr}");
    // The report has to name which switches, or it sends nobody anywhere.
    assert!(stderr.contains("DSH") && stderr.contains("DSL"), "{stderr}");
}

#[test]
fn a_correctly_gated_leg_raises_nothing() {
    // The same deck with the low gate as NOT(high gate). Without this the test above would pass
    // for a rule that simply always fires.
    let good = SHOOT_THROUGH_LEG.replace(
        "PWML kind=pwm freq=20000 in=COMP",
        "PWML kind=sum inputs=ONE,PWMH signs=1,-1",
    );
    let netlist = write(
        "leg_ok.cir",
        &format!(
            "{good}\n\
             R_leg kind=rule type=forbidden_state switches=DSH,DSL severity=fatal action=stop\n"
        ),
    );
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "2e-4",
        "--dt",
        "1e-7",
    ]);
    assert_eq!(output.status.code(), Some(0), "a clean leg must pass");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("0 violated, 1 passed"), "{stderr}");
}

#[test]
fn a_run_a_rule_stopped_resumes_after_the_rule_is_adjusted() {
    // The workflow the feature exists for, end to end:
    //   1. the run stops on a rule and leaves a checkpoint;
    //   2. the rule is relaxed -- which must NOT invalidate that checkpoint, since the circuit
    //      has not changed. This is why rule cards are stripped before the deck is hashed.
    //   3. the resumed run continues, and the two halves rejoin into exactly the trace an
    //      uninterrupted run produces.
    let whole_netlist = write("whole.cir", BUCK);
    let whole = run(&[
        whole_netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "3e-4",
        "--dt",
        "1e-7",
    ]);
    assert!(whole.status.success());
    let whole_csv = String::from_utf8_lossy(&whole.stdout).to_string();

    let stopping = write(
        "stop.cir",
        &format!(
            "{BUCK}\nR_isat kind=rule type=above target=I(L1) limit=3 unit=A severity=fatal action=stop\n"
        ),
    );
    let checkpoint = scratch("stop.ckpt");
    let first = run(&[
        stopping.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "3e-4",
        "--dt",
        "1e-7",
        "--checkpoint-out",
        checkpoint.to_str().unwrap(),
    ]);
    assert_eq!(first.status.code(), Some(3), "the rule should stop it");
    assert!(
        checkpoint.exists(),
        "a stopped run must still leave its state"
    );
    let part1 = String::from_utf8_lossy(&first.stdout).to_string();
    assert!(
        part1.lines().count() > 1 && part1.lines().count() < whole_csv.lines().count(),
        "it should stop partway, not at once or not at all"
    );

    // Only the limit changes; the circuit is untouched.
    let relaxed = write(
        "relaxed.cir",
        &format!(
            "{BUCK}\nR_isat kind=rule type=above target=I(L1) limit=50 unit=A severity=warning\n"
        ),
    );
    let second = run(&[
        relaxed.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "3e-4",
        "--dt",
        "1e-7",
        "--resume",
        checkpoint.to_str().unwrap(),
    ]);
    assert_eq!(
        second.status.code(),
        Some(0),
        "the resume must load -- editing a rule is not editing the deck.\nstderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );
    let part2 = String::from_utf8_lossy(&second.stdout).to_string();

    let rejoined: String = part1
        .lines()
        .chain(part2.lines().skip(1))
        .map(|line| format!("{line}\n"))
        .collect();
    assert_eq!(
        rejoined, whole_csv,
        "stopping on a rule and resuming must reproduce the uninterrupted run exactly"
    );
}
