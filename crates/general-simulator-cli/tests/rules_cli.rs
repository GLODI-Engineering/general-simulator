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
    // `<target>.<rule>`, the same label the event lines and the summary table print. The
    // target is part of it because a rule's own name is not always unique or self-explanatory:
    // a rating written on a device line is named for its field, so "stopped by rule 'i_max'"
    // would leave the reader to guess which component.
    assert!(
        stderr.contains("stopped by rule 'DSH+DSL.R_leg'"),
        "{stderr}"
    );
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

// -------------------------------------------------------------------------------------------
// Ratings written on the device line. The specification's second syntax for the same thing,
// compiling to the same `Rule` objects -- so what is tested here is the compilation and the
// stripping, not the engine, which the cards above already cover. See #46.
// -------------------------------------------------------------------------------------------

/// The same buck, with the inductor's rating written where the inductor is declared.
fn buck_rated(rating: &str) -> String {
    BUCK.replace("L1 sw out 22e-6", &format!("L1 sw out 22e-6 {rating}"))
}

#[test]
fn a_rating_on_the_device_line_reports_exactly_as_a_card_does() {
    let netlist = write("rated.cir", &buck_rated("i_max=3.5"));
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "3e-4",
        "--dt",
        "1e-7",
    ]);
    // A rating defaults to `error`, so the exit code says the run is not to be trusted.
    assert_eq!(
        output.status.code(),
        Some(2),
        "a rating is an error by default"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    // `L1.i_max`: the component, then the rating. A reader should not have to look up which
    // inductor, and a GUI highlighting the schematic needs the component name, not `I(L1)`.
    assert!(stderr.contains("L1.i_max"), "{stderr}");

    let events = fs::read_to_string(netlist.with_extension("events.jsonl")).expect("the log");
    assert!(
        events.contains("\"rule\":\"i_max\"") && events.contains("\"target\":\"L1\""),
        "{events}"
    );
    assert!(events.contains("\"kind\":\"enter\""), "{events}");
}

#[test]
fn an_unexceeded_rating_passes_rather_than_being_ignored() {
    // Without this the test above would pass for a rating that always fires -- and, worse, for
    // one that is accepted and never evaluated, which is what the device-line form risked.
    let netlist = write("rated_ok.cir", &buck_rated("i_max=50"));
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "3e-4",
        "--dt",
        "1e-7",
    ]);
    assert_eq!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("0 violated, 1 passed"), "{stderr}");
}

#[test]
fn two_ratings_on_one_line_are_two_independent_thresholds() {
    // The datasheet case: a part is rated for 3.5 A and saturates at 50 A. One line, two
    // limits, and they must not be conflated.
    let netlist = write("rated_two.cir", &buck_rated("i_max=3.5 i_sat=50"));
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "3e-4",
        "--dt",
        "1e-7",
    ]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("2 rule(s): 1 violated, 1 passed"),
        "{stderr}"
    );
    assert!(stderr.contains("L1.i_sat"), "{stderr}");
}

#[test]
fn a_rating_is_a_limit_on_magnitude_not_on_signed_value() {
    // A reversed inductor, so the branch current is negative throughout. `above(limit)` would
    // watch a 4 A reverse current go past a 3.5 A rating without a word; a bridge inductor's
    // current is bipolar and this is the common case, not a corner one.
    let netlist = write(
        "reversed.cir",
        "VIN vin 0 12\nR1 vin a 1\nL1 0 a 22e-6 i_max=3.5\n",
    );
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "1e-5",
        "--dt",
        "1e-7",
    ]);
    assert_eq!(
        output.status.code(),
        Some(2),
        "a reverse overload is an overload"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("-4.3831 A"), "{stderr}");
}

#[test]
fn on_violation_stops_the_run_and_the_message_names_the_component() {
    let netlist = write("rated_stop.cir", &buck_rated("i_max=3.5 on_violation=stop"));
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "3e-4",
        "--dt",
        "1e-7",
    ]);
    assert_eq!(output.status.code(), Some(3), "stopped by a rating");
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Not "stopped by rule 'i_max'", which would leave the reader to guess the component.
    assert!(stderr.contains("stopped by rule 'L1.i_max'"), "{stderr}");
}

#[test]
fn a_rating_on_a_device_with_no_branch_current_is_refused_at_load() {
    // The alternative is the failure this phase exists to avoid: a rating accepted and never
    // evaluated, which in the summary table is indistinguishable from one never exceeded.
    let netlist = write("rated_r.cir", "VIN vin 0 12\nR1 vin 0 50 i_max=2\n");
    let output = run(&[netlist.to_str().unwrap(), "--mode", "transient"]);
    assert_ne!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("device R does not have one"), "{stderr}");
    assert!(stderr.contains("only V, L, E and H"), "{stderr}");
}

#[test]
fn raising_a_rating_still_loads_the_checkpoint_of_the_run_it_stopped() {
    // The same workflow the card test above covers, through the other syntax -- and the reason
    // a rating is cut out of the device line rather than left on it. Raising the limit is the
    // first thing anyone does after a run stops on it; if that invalidated the checkpoint, the
    // two features would be unusable together.
    let whole_netlist = write("rating_whole.cir", BUCK);
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

    let stopping = write("rating_stop.cir", &buck_rated("i_max=3 on_violation=stop"));
    let checkpoint = scratch("rating.ckpt");
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
    assert_eq!(first.status.code(), Some(3), "the rating should stop it");
    let part1 = String::from_utf8_lossy(&first.stdout).to_string();
    assert!(
        part1.lines().count() > 1 && part1.lines().count() < whole_csv.lines().count(),
        "it should stop partway, not at once or not at all"
    );

    // Only the number on the device line changes. The circuit is untouched.
    let relaxed = write("rating_relaxed.cir", &buck_rated("i_max=50"));
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
        "editing a rating is not editing the deck.\nstderr: {}",
        String::from_utf8_lossy(&second.stderr)
    );

    let rejoined: String = part1
        .lines()
        .chain(String::from_utf8_lossy(&second.stdout).lines().skip(1))
        .map(|line| format!("{line}\n"))
        .collect();
    assert_eq!(
        rejoined, whole_csv,
        "stopping on a rating and resuming must reproduce the uninterrupted run exactly"
    );
}

// -------------------------------------------------------------------------------------------
// `type=switch_timing`: never commutated, minimum on/off time, switching frequency. See #52.
// -------------------------------------------------------------------------------------------

/// The buck with its gate wired to a constant 0, so the switch never turns on. The converter
/// still charges through the diode and still settles: the waveform of this deck is the reason
/// the check has to exist, because nothing about it looks wrong.
const DEAD_GATE: &str = "\
VIN vin 0 12
DSW vin sw idealswitchmodel
DSW kind=ideal_switch r_on=0.05 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.5 g_on=5 gate=block ctrl=GATE
D1 0 sw dmodel
D1 kind=ideal_diode g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.4 g_on=5
L1 sw out 22e-6
C1 out 0 47e-6 ic=3
RL out 0 4
DEAD kind=const value=0
GATE kind=sig2phys domain=voltage in=DEAD
";

#[test]
fn a_switch_that_never_commutated_is_reported() {
    let netlist = write(
        "dead_gate.cir",
        &format!("{DEAD_GATE}T_sw kind=rule type=switch_timing switches=DSW severity=error\n"),
    );
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "1e-4",
        "--dt",
        "1e-7",
    ]);
    assert_eq!(output.status.code(), Some(2), "a dead gate is an error");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("never commutated"), "{stderr}");
    assert!(stderr.contains("stayed open"), "{stderr}");
}

#[test]
fn a_switch_that_does_commutate_passes_the_same_rule() {
    // The same rule on the working buck. Without this, the test above would pass for a check
    // that reports every switch it is pointed at.
    let netlist = write(
        "live_gate.cir",
        &format!("{BUCK}T_sw kind=rule type=switch_timing switches=DSW severity=error\n"),
    );
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "1e-4",
        "--dt",
        "1e-7",
    ]);
    assert_eq!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("0 violated, 1 passed"), "{stderr}");
}

#[test]
fn an_on_pulse_shorter_than_the_device_allows_is_reported_with_its_length() {
    // The buck's duty is about 0.44 at 100 kHz, so the on-time is roughly 4.4 us. Demanding
    // 6 us reports it; the number in the log is the pulse that was actually delivered.
    let netlist = write(
        "min_on.cir",
        &format!(
            "{BUCK}T_sw kind=rule type=switch_timing switches=DSW min_on=6u severity=warning\n"
        ),
    );
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "5e-5",
        "--dt",
        "1e-7",
    ]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("DSW was on for"), "{stderr}");
    assert!(stderr.contains("6.0000e-6 s required"), "{stderr}");

    let events = fs::read_to_string(netlist.with_extension("events.jsonl")).expect("the log");
    assert!(events.contains("\"kind\":\"instant\""), "{events}");
    assert!(events.contains("\"unit\":\"s\""), "{events}");
}

#[test]
fn a_switching_frequency_is_measured_and_held_to_its_band() {
    // The modulator is a 100 kHz PWM. Measured from the resolved transitions, which is the
    // point: with a closed loop the frequency is an outcome, not a setting.
    let netlist = write(
        "freq.cir",
        &format!(
            "{BUCK}T_f kind=rule type=switch_timing switches=DSW f_min=200k \
             require_commutation=no severity=warning\n"
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
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("below 2.0000e5 Hz"), "{stderr}");
    // 100 kHz asked for, 100 kHz measured: the figure has to be trustworthy or the band is
    // meaningless. A 1 kHz window on a 100 kHz measurement is a 1% tolerance.
    assert!(
        stderr.contains("1.0003e5 Hz") || stderr.contains("1.0000e5 Hz"),
        "{stderr}"
    );

    // And inside a band that contains it, the same deck passes.
    let ok = write(
        "freq_ok.cir",
        &format!(
            "{BUCK}T_f kind=rule type=switch_timing switches=DSW f_min=90k f_max=110k \
             severity=warning\n"
        ),
    );
    let output = run(&[
        ok.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "3e-4",
        "--dt",
        "1e-7",
    ]);
    assert_eq!(output.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("0 violated, 1 passed"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn require_commutation_rejects_a_value_that_is_not_yes_or_no() {
    let netlist = write(
        "bad_flag.cir",
        &format!(
            "{BUCK}T_sw kind=rule type=switch_timing switches=DSW require_commutation=maybe\n"
        ),
    );
    let output = run(&[netlist.to_str().unwrap(), "--mode", "transient"]);
    assert_ne!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("require_commutation must be yes or no"),
        "{stderr}"
    );
}

// -------------------------------------------------------------------------------------------
// The static check, which needs no rule at all: a gate provably pinned below the threshold is
// reported before the run starts. See #54.
// -------------------------------------------------------------------------------------------

#[test]
fn a_gate_pinned_at_zero_is_reported_without_any_rule_being_written() {
    // No `kind=rule` line in this deck. The point of a static check is that it costs the user
    // nothing: the mistake is found before a transient that may take minutes.
    let netlist = write("static_dead.cir", DEAD_GATE);
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "2e-5",
        "--dt",
        "1e-7",
    ]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("can never conduct"), "{stderr}");
    assert!(stderr.contains("fixed at 0 by 'DEAD'"), "{stderr}");
    // A warning, not an error: a deliberately disabled leg is a legitimate deck, and refusing
    // to run would be the wrong answer to a gate someone pinned at zero on purpose.
    assert_eq!(
        output.status.code(),
        Some(0),
        "a warning does not fail a run"
    );
}

#[test]
fn a_deck_whose_gates_are_driven_by_a_modulator_says_nothing() {
    // The restraint half. A PWM's output is not foldable from its declaration, and a warning
    // about something merely unproven would train its reader to ignore the ones that matter.
    let netlist = write("static_live.cir", BUCK);
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "2e-5",
        "--dt",
        "1e-7",
    ]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("can never conduct"),
        "a working buck must be silent: {stderr}"
    );
}
