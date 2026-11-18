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
DSW kind=ideal_switch r_on=0.05 body_g_breakdown=0 body_v_breakdown=-100 body_g_off=0 body_v_f=0.5 body_g_on=5 gate=block ctrl=GATE
D1 0 sw dmodel
D1 kind=ideal_diode g_breakdown=0 v_breakdown=-100 g_off=0 v_f=0.4 g_on=5
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
DSH kind=ideal_switch r_on=0.01 body_g_breakdown=0 body_v_breakdown=-1000 body_g_off=0 body_v_f=0.5 body_g_on=100 gate=block ctrl=GH
DSL mid 0 idealswitchmodel
DSL kind=ideal_switch r_on=0.01 body_g_breakdown=0 body_v_breakdown=-1000 body_g_off=0 body_v_f=0.5 body_g_on=100 gate=block ctrl=GL
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
        &format!("{BUCK}\nR_isat kind=rule type=above target=I(L1) max=3 unit=A severity=error\n"),
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
            "{BUCK}\nR_isat kind=rule type=above target=I(L1) max=3 unit=A severity=fatal action=stop\n"
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
            "{BUCK}\nR_isat kind=rule type=above target=I(L1) max=50 unit=A severity=warning\n"
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
DSW kind=ideal_switch r_on=0.05 body_g_breakdown=0 body_v_breakdown=-100 body_g_off=0 body_v_f=0.5 body_g_on=5 gate=block ctrl=GATE
D1 0 sw dmodel
D1 kind=ideal_diode g_breakdown=0 v_breakdown=-100 g_off=0 v_f=0.4 g_on=5
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

// -------------------------------------------------------------------------------------------
// Conduction checks through the binary. See #53.
// -------------------------------------------------------------------------------------------

/// A synchronous buck: a high-side switch and a low-side sync rectifier driven by the
/// complementary outputs of one PWM, with 300 ns of dead time inserted on each edge.
///
/// A 0 V source sits in series with each switch. That is how a switch's own current becomes
/// observable -- only V, L, E and H carry a branch-current unknown, and `general-mna` says so
/// when asked for a switch's current directly.
const SYNC_BUCK: &str = "\
VIN vin 0 12
VSH vin sh 0
DSH sh mid idealswitchmodel
DSH kind=ideal_switch r_on=0.02 body_g_breakdown=0 body_v_breakdown=-100 body_g_off=0 body_v_f=0.4 body_g_on=50 gate=block ctrl=GH
VSL mid sl 0
DSL sl 0 idealswitchmodel
DSL kind=ideal_switch r_on=0.02 body_g_breakdown=0 body_v_breakdown=-100 body_g_off=0 body_v_f=0.4 body_g_on=50 gate=block ctrl=GL
L1 mid out 22e-6
C1 out 0 47e-6 ic=5
RL out 0 4
DUTY kind=const value=0.45
PWM1 kind=pwm freq=100000 in=DUTY red=300e-9 fed=300e-9
GH kind=sig2phys domain=voltage in=PWM1
GL kind=sig2phys domain=voltage in=PWM1_comp
";

#[test]
fn a_body_diode_carrying_the_dead_time_current_is_reported_with_its_duration() {
    // This test measures the dead time twice, by two independent routes, and they must agree:
    // 300 ns was inserted into the modulator with `fed=`, and what comes back is the length of
    // the interval during which the low-side switch was commanded off and its body diode was
    // resolved into conduction. If the rule were reading the wrong thing, the two numbers
    // would not match to three digits.
    let netlist = write(
        "sync_body.cir",
        &format!(
            "{SYNC_BUCK}B_dsl kind=rule type=body_diode switches=DSL currents=I(VSL) \
             i_min=0.5 severity=warning\n"
        ),
    );
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "3e-5",
        "--dt",
        "1e-8",
    ]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("body diode conducting in DSL"), "{stderr}");
    assert!(
        stderr.contains("302.3458 ns") || stderr.contains("302."),
        "the measured interval should be the 300 ns dead time: {stderr}"
    );
}

#[test]
fn a_threshold_above_the_dead_time_current_keeps_ordinary_freewheeling_quiet() {
    // Without this the test above would pass for a rule that reports any commanded-off switch.
    let netlist = write(
        "sync_body_ok.cir",
        &format!(
            "{SYNC_BUCK}B_dsl kind=rule type=body_diode switches=DSL currents=I(VSL) \
             i_min=50 severity=warning\n"
        ),
    );
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "3e-5",
        "--dt",
        "1e-8",
    ]);
    assert_eq!(output.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("0 violated, 1 passed"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_slope_bound_reports_the_slope_the_circuit_actually_produced() {
    // The buck's inductor sees about 9 V across 22 uH at start-up, so di/dt is about
    // 4.1e5 A/s. The figure in the log is checked against that derivation rather than against
    // itself: a bound of 2e5 must report roughly 4.1e5, not merely something.
    let netlist = write(
        "slew.cir",
        &format!(
            "{BUCK}R_slew kind=rule type=rate_of_change target=I(L1) max_slope=2e5 \
             unit=A/s severity=warning\n"
        ),
    );
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
    assert!(stderr.contains("limit 200000.0000A/s"), "{stderr}");
    assert!(stderr.contains("40"), "about 4.1e5 A/s expected: {stderr}");

    // And a bound the circuit cannot reach passes, so the rule is not simply always firing.
    let ok = write(
        "slew_ok.cir",
        &format!(
            "{BUCK}R_slew kind=rule type=rate_of_change target=I(L1) max_slope=1e9 \
             unit=A/s severity=warning\n"
        ),
    );
    let output = run(&[
        ok.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "2e-5",
        "--dt",
        "1e-7",
    ]);
    assert_eq!(output.status.code(), Some(0));
}

#[test]
fn a_commutation_order_the_bridge_keeps_passes() {
    let netlist = write(
        "order_ok.cir",
        &format!("{SYNC_BUCK}C_ord kind=rule type=commutation_order sequence=DSH,DSL\n"),
    );
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "3e-5",
        "--dt",
        "1e-8",
    ]);
    assert_eq!(output.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("0 violated, 1 passed"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_conduction_cards_reject_what_they_cannot_act_on() {
    // A current per switch, because a switch's own current is not an MNA unknown and the rule
    // cannot invent one.
    let mismatched = write(
        "order_bad.cir",
        &format!(
            "{SYNC_BUCK}B_x kind=rule type=body_diode switches=DSH,DSL currents=I(VSH) i_min=1\n"
        ),
    );
    let output = run(&[mismatched.to_str().unwrap(), "--mode", "transient"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("lists 1 name(s) for 2 switch(es)"),
        "{stderr}"
    );

    // An order of one is not an order.
    let single = write(
        "order_one.cir",
        &format!("{SYNC_BUCK}C_ord kind=rule type=commutation_order sequence=DSH\n"),
    );
    let output = run(&[single.to_str().unwrap(), "--mode", "transient"]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("needs at least two switches"), "{stderr}");
}

// -------------------------------------------------------------------------------------------
// The finite check, which is on by default because nobody suspects a NaN. See #59.
// -------------------------------------------------------------------------------------------

#[test]
fn a_run_that_goes_non_finite_is_abandoned_and_says_which_quantity_went_first() {
    // `sqrt` of a negative constant is a NaN, and nothing in the block library refuses it.
    let netlist = write(
        "nan.cir",
        "VIN vin 0 12\nR1 vin a 1\nL1 a 0 22e-6\nNEG kind=const value=-1\nBAD kind=sqrt in=NEG\n",
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
    // Its own code: 3 means a rule did its job on a run that behaved as the deck said it
    // would, and 4 means a run with no result at all. A script has to be able to tell those
    // apart.
    assert_eq!(
        output.status.code(),
        Some(4),
        "a non-finite run has its own code"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("'BAD' is not a finite number"), "{stderr}");
    assert!(stderr.contains("not a number"), "{stderr}");
    assert!(stderr.contains("abandoned at step 0"), "{stderr}");
}

#[test]
fn a_healthy_run_is_not_touched_by_the_finite_check() {
    // It runs on every step of every deck, so a false positive would break everything.
    let netlist = write("finite_ok.cir", "VIN vin 0 12\nR1 vin a 1\nL1 a 0 22e-6\n");
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "1e-5",
        "--dt",
        "1e-7",
    ]);
    assert_eq!(output.status.code(), Some(0));
    assert!(
        String::from_utf8_lossy(&output.stderr).is_empty(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_rule_on_a_non_finite_run_is_not_reported_as_passed() {
    // The failure this check exists to remove. With a rule on the deck, the summary must not
    // claim the limit held: it was never readable.
    let netlist = write(
        "nan_rule.cir",
        "VIN vin 0 12\nR1 vin a 1\nL1 a 0 22e-6\nNEG kind=const value=-1\n\
         BAD kind=sqrt in=NEG\n\
         R_bad kind=rule type=above target=BAD max=1 severity=error\n",
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
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("1 passed"),
        "a rule that could not read its quantity must not pass: {stderr}"
    );
}

// -------------------------------------------------------------------------------------------
// `type=per_period`: the measures that only mean something averaged over a switching period.
// See #60.
// -------------------------------------------------------------------------------------------

#[test]
fn a_volt_second_imbalance_is_reported_and_decays_as_the_converter_settles() {
    // The check this phase was asked for, and its own validation. Volt-seconds across an
    // inductor balance only in steady state, so during a buck's start-up the integral per
    // period is large and must shrink monotonically as the output approaches its reference.
    // A measure that did not do that would be measuring something else.
    let netlist = write(
        "vs.cir",
        &format!(
            "{BUCK}P_vs kind=rule type=per_period target=V(sw,out) measure=integral \
             period=10e-6 min=-2e-6 max=2e-6 unit=Vs severity=warning\n"
        ),
    );
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "6e-4",
        "--dt",
        "1e-8",
    ]);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let found: Vec<f64> = stderr
        .lines()
        .filter(|line| line.contains("P_vs") && line.contains("at "))
        .filter_map(|line| {
            let after = line.split("at ").nth(1)?;
            after.trim().split("Vs").next()?.trim().parse::<f64>().ok()
        })
        .collect();
    assert!(found.len() >= 5, "several periods expected: {stderr}");
    assert!(
        found[0] > found[found.len() - 1] * 3.0,
        "the imbalance must shrink as the converter settles: {found:?}"
    );
    // And it is printed in a form that can be read at this magnitude: `0.0000Vs` would be a
    // rule that fired without saying what it found.
    assert!(
        stderr.contains("e-5Vs") || stderr.contains("e-6Vs"),
        "{stderr}"
    );
}

#[test]
fn a_per_period_rms_reports_the_rms_and_not_the_peak() {
    // The buck's inductor current ramps between roughly 2 and 4 A once running, so its RMS per
    // period is well above a 1 A bound and well below the peak -- which is the distinction that
    // makes RMS worth measuring for a component rating.
    let netlist = write(
        "prms.cir",
        &format!(
            "{BUCK}P_rms kind=rule type=per_period target=I(L1) measure=rms period=10e-6 \
             max=1.0 unit=A severity=warning\n"
        ),
    );
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "5e-5",
        "--dt",
        "1e-8",
    ]);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("RMS of I(L1)"), "{stderr}");
    assert!(stderr.contains("limit 1.0000A"), "{stderr}");
}

#[test]
fn a_per_period_card_insists_on_knowing_what_a_period_is() {
    for (card, expected) in [
        (
            "P kind=rule type=per_period target=I(L1) measure=rms max=1",
            "needs 'period=<seconds>' or 'sync=<switch>'",
        ),
        (
            "P kind=rule type=per_period target=I(L1) measure=rms period=10u sync=DSW max=1",
            "'period' and 'sync' both given",
        ),
        (
            "P kind=rule type=per_period target=I(L1) measure=rms period=10u",
            "needs 'min' or 'max'",
        ),
        (
            "P kind=rule type=per_period target=I(L1) measure=median period=10u max=1",
            "unknown measure 'median'",
        ),
    ] {
        let netlist = write("pbad.cir", &format!("{BUCK}{card}\n"));
        let output = run(&[netlist.to_str().unwrap(), "--mode", "transient"]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains(expected), "for {card}:\n{stderr}");
    }
}

// ---------------------------------------------------------------------------
// Declared ports and efficiency
// ---------------------------------------------------------------------------

/// A buck with a port on each side: the input port on the source, the output port on a
/// zero-volt sense source in line with the load.
///
/// The two declarations differ by the leading minus on `i=`, and that is the whole sign
/// convention. A source's branch current leaves its positive terminal, so the input port
/// negates it; a series sense element's enters, so the output port does not. See `rules::Port`
/// for the measurement that establishes this.
const BUCK_WITH_PORTS: &str = "\
VIN in 0 12
DSW in sw idealswitchmodel
DSW kind=ideal_switch r_on=0.01 body_g_breakdown=0 body_v_breakdown=-100 body_g_off=0 body_v_f=0.5 body_g_on=100 gate=block ctrl=GATE
D1 0 sw idealdiodemodel
D1 kind=ideal_diode g_breakdown=0 v_breakdown=-100 g_off=0 v_f=0.4 g_on=100
L1 sw out 1e-3
C1 out 0 1e-4
VSENSE out load 0
R1 load 0 5
DUTY kind=const value=0.4
PWM1 kind=pwm freq=50000 in=DUTY
GATE kind=sig2phys domain=voltage in=PWM1
P_IN kind=port v=(in,0) i=-VIN
P_OUT kind=port v=(out,0) i=VSENSE
";

/// A dependent source stamped with a gain that creates energy: the output port carries 40 times
/// the input port's power. This is the mistake the rule exists for -- a transformer with the
/// wrong turns ratio, a dependent source with the wrong gain -- and nothing in any single
/// waveform is out of range.
const OVER_UNITY: &str = "\
VIN in 0 12
R1 in mid 1
R3 mid 0 100
E1 out 0 mid 0 2
R2 out 0 10
P_IN kind=port v=(in,0) i=-VIN
P_OUT kind=port v=(out,0) i=-E1
";

#[test]
fn a_real_converter_efficiency_sits_inside_its_band() {
    // The averaging window is 2 ms, not one switching period, and that is the whole lesson of
    // this test. See `a_window_shorter_than_the_output_ring_exceeds_unity_legitimately` for the
    // failure it avoids and why it is not a bug.
    let netlist = write(
        "eta_pass.cir",
        &format!(
            "{BUCK_WITH_PORTS}\
             ETA kind=rule type=efficiency in=P_IN out=P_OUT min=0 max=1 period=2e-3 severity=error\n"
        ),
    );
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "8e-3",
        "--dt",
        "2e-7",
        "--out-every",
        "1000000",
    ]);
    let text = fs::read_to_string(netlist.with_extension("events.txt")).expect("the text log");
    assert!(
        text.contains("P_OUT/P_IN.ETA               passed"),
        "a buck averaged over its ring period should never look over-unity:\n{text}"
    );
    assert_eq!(output.status.code(), Some(0), "{text}");
}

#[test]
fn a_window_shorter_than_the_output_ring_exceeds_unity_legitimately() {
    // Found by running the rule on a real buck rather than reasoning about it: with a
    // one-switching-period window this converter reports efficiencies up to 1.29, and with a
    // 1 ms window 1.36. Neither is energy creation and neither is a defect in the rule.
    //
    // $\eta = \overline{p_\text{out}} / \overline{p_\text{in}}$ accounts for no *stored* energy.
    // While the output filter rings, the capacitor spends half of each ring cycle delivering
    // into the load energy that the input supplied on an earlier cycle, so the load genuinely
    // receives more power than the source provides. Here $L = 1$ mH and $C = 100$ uF, so
    // $f_0 = 1 / (2\pi\sqrt{LC}) = 503$ Hz and the ring period is 1.99 ms -- which is why the
    // passing test above averages over 2 ms and this one, at half that, reports.
    //
    // The rule for anyone writing one of these: the window must be long compared with the
    // slowest energy-storage cycle in the circuit, not with the switching period.
    let netlist = write(
        "eta_short_window.cir",
        &format!(
            "{BUCK_WITH_PORTS}\
             ETA kind=rule type=efficiency in=P_IN out=P_OUT max=1 period=1e-3 severity=error\n"
        ),
    );
    run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "2.2e-3",
        "--dt",
        "2e-7",
        "--out-every",
        "1000000",
    ]);
    let text = fs::read_to_string(netlist.with_extension("events.txt")).expect("the text log");
    assert!(
        text.contains("ERROR P_OUT/P_IN.ETA"),
        "half a ring period should report the capacitor's discharge as over-unity:\n{text}"
    );
    // The finding must carry both means, because the only way a reader can tell this apart from
    // a genuine over-unity stamp is by seeing that the output power exceeds the input by a
    // little and transiently, rather than by a factor.
    assert!(
        text.contains("mean P_OUT / mean P_IN ="),
        "the finding should show both averages:\n{text}"
    );
}

#[test]
fn a_stamp_that_creates_energy_is_caught_and_stops_the_run() {
    let netlist = write(
        "eta_over_unity.cir",
        &format!(
            "{OVER_UNITY}\
             ETA kind=rule type=efficiency in=P_IN out=P_OUT max=1 period=1e-5 severity=error action=stop\n"
        ),
    );
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "1e-4",
        "--dt",
        "1e-6",
        "--out-every",
        "100000",
    ]);
    let text = fs::read_to_string(netlist.with_extension("events.txt")).expect("the text log");
    // The findings go to the log; the stop notice goes to the console, which is where someone
    // watching the run reads it.
    let console = String::from_utf8_lossy(&output.stderr);
    assert!(
        console.contains("stopped by rule 'P_OUT/P_IN.ETA'"),
        "an over-unity stamp should stop the run:\n{console}"
    );
    // The report has to say *how far* over unity, not merely that a limit was passed: 39.6 is
    // the gain squared over the load ratio, and a reader needs the number to find the stamp.
    assert!(
        text.contains("ERROR P_OUT/P_IN.ETA") && text.contains("limit 1.0000"),
        "the finding should name the ratio and the limit:\n{text}"
    );
    assert_eq!(output.status.code(), Some(3), "stopped by a rule:\n{text}");
}

#[test]
fn a_port_declared_backwards_reports_a_negative_efficiency_rather_than_inverting_it() {
    // `i=VIN` without the minus on an input port: the source's branch current is in the passive
    // convention, so the port's power comes out negative. The rule reports it instead of
    // quietly producing a plausible-looking positive efficiency, which is the only reason
    // `min=0` is worth writing.
    let netlist = write(
        "eta_backwards.cir",
        &format!(
            "{}\
             ETA kind=rule type=efficiency in=P_IN out=P_OUT min=0 sync=DSW severity=error\n",
            BUCK_WITH_PORTS.replace("i=-VIN", "i=VIN")
        ),
    );
    run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "5e-4",
        "--dt",
        "2e-7",
        "--out-every",
        "100000",
    ]);
    let text = fs::read_to_string(netlist.with_extension("events.txt")).expect("the text log");
    // A negative *denominator* is the signature: the input port is reporting power flowing the
    // wrong way. Matched on the detail rather than on the ratio's own text, because early in
    // the run the ratio is small enough to be printed in exponent form.
    assert!(
        text.contains("ERROR P_OUT/P_IN.ETA")
            && text.contains("mean P_IN = ")
            && text.contains("/ -"),
        "a backwards port should report a negative input power:\n{text}"
    );
}

#[test]
fn a_period_whose_input_power_averages_to_zero_is_skipped_not_reported() {
    // An unpowered circuit: the denominator is zero every period. A ratio against nothing is
    // not a finding about the circuit, and a converter whose input has not arrived yet would
    // otherwise be reported for being off.
    let netlist = write(
        "eta_zero.cir",
        "VIN in 0 0
R1 in mid 1
R2 mid 0 10
P_IN kind=port v=(in,0) i=-VIN
P_OUT kind=port v=(mid,0) i=-VIN
ETA kind=rule type=efficiency in=P_IN out=P_OUT max=1 min=0 period=1e-5 severity=error
",
    );
    run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "1e-4",
        "--dt",
        "1e-6",
        "--out-every",
        "100000",
    ]);
    let text = fs::read_to_string(netlist.with_extension("events.txt")).expect("the text log");
    assert!(
        text.contains("P_OUT/P_IN.ETA               passed"),
        "a zero denominator should be skipped, not reported:\n{text}"
    );
}

#[test]
fn a_port_power_converges_to_the_product_the_netlist_can_compute() {
    // The mechanism the port object replaces: probe the node and the branch into the signal
    // domain and multiply them with a `kind=product` block. The two do *not* agree exactly, and
    // finding out why is what justifies the port object existing at all -- a block output is
    // evaluated with a one-step lag, so the netlist-computed product trails the port by one
    // step and converges to it as the step shrinks.
    //
    // Asserted as first-order convergence rather than equality, because equality is false. With
    // the startup skipped (`after=`, since the first periods are dominated by a waveform rising
    // from zero where the lag is a large fraction of the value), the measured deviations from
    // unity were 2.37% at dt=4e-7, 1.18% at 2e-7, and inside 1% from 1e-7 down -- halving with
    // the step, which is what a one-step lag does and what no other explanation would.
    let deck = "\
VIN in 0 12
DSW in sw idealswitchmodel
DSW kind=ideal_switch r_on=0.01 body_g_breakdown=0 body_v_breakdown=-100 body_g_off=0 body_v_f=0.5 body_g_on=100 gate=block ctrl=GATE
D1 0 sw idealdiodemodel
D1 kind=ideal_diode g_breakdown=0 v_breakdown=-100 g_off=0 v_f=0.4 g_on=100
L1 sw out 1e-3
C1 out 0 1e-4
R1 out 0 5
DUTY kind=const value=0.4
PWM1 kind=pwm freq=50000 in=DUTY
GATE kind=sig2phys domain=voltage in=PWM1
VPROBE kind=phys2sig node=in
IPROBE kind=phys2sig branch=VIN
NEG kind=const value=-1
P_PRODUCT kind=product inputs=VPROBE,IPROBE,NEG
P_IN kind=port v=(in,0) i=-VIN
";
    // A band that the coarse step fails and the fine step passes is the convergence assertion:
    // one deck, one rule, two step sizes, opposite outcomes.
    let netlist = write(
        "eta_agree.cir",
        &format!(
            "{deck}\
             CHK kind=rule type=efficiency in=P_PRODUCT out=P_IN min=0.99 max=1.01 \
             period=1e-5 after=8e-5 severity=error\n"
        ),
    );
    let at = |dt: &str| -> String {
        let output = run(&[
            netlist.to_str().unwrap(),
            "--mode",
            "transient",
            "--tfinal",
            "2e-4",
            "--dt",
            dt,
            "--out-every",
            "2000000",
        ]);
        let _ = output;
        fs::read_to_string(netlist.with_extension("events.txt")).expect("the text log")
    };
    let coarse = at("4e-7");
    assert!(
        coarse.contains("ERROR P_IN/P_PRODUCT.CHK"),
        "at dt=4e-7 the lagging product should sit outside a 1% band:\n{coarse}"
    );
    let fine = at("1e-7");
    assert!(
        fine.contains("P_IN/P_PRODUCT.CHK") && fine.contains("passed"),
        "at dt=1e-7 the product should have converged into the 1% band:\n{fine}"
    );
}

// ---------------------------------------------------------------------------
// Conduction loss
// ---------------------------------------------------------------------------

/// A synchronous buck with a real complement and a real dead time.
///
/// `PWMH_comp` rather than `PWM(1-d)`: a complement computed as one-minus-the-duty makes both
/// switches conduct together on every cycle, which is what `SHOOT_THROUGH_LEG` above exists to
/// demonstrate. Writing this deck that way first gave a conduction loss of 721 W in a 12 V
/// converter -- correct arithmetic on a shoot-through circuit, since 12 V across two 20 mOhm
/// channels is 300 A and `300^2 * 0.02 * 0.4` is exactly the 721 W reported.
const SYNC_BUCK_CHANNELS: &str = "\
VIN in 0 12
DSH in mid idealswitchmodel
DSH kind=ideal_switch r_on=0.02 body_g_breakdown=0 body_v_breakdown=-100 body_g_off=0 body_v_f=0.5 body_g_on=100 gate=block ctrl=GH
DSL mid 0 idealswitchmodel
DSL kind=ideal_switch r_on=0.02 body_g_breakdown=0 body_v_breakdown=-100 body_g_off=0 body_v_f=0.5 body_g_on=100 gate=block ctrl=GL
L1 mid out 1e-3
C1 out 0 1e-4
R1 out 0 5
DUTY kind=const value=0.4
PWMH kind=pwm freq=50000 in=DUTY red=200e-9 fed=200e-9
GH kind=sig2phys domain=voltage in=PWMH
GL kind=sig2phys domain=voltage in=PWMH_comp
";

#[test]
fn a_switchs_computed_current_agrees_with_a_measured_one() {
    // The premise the whole conduction-loss path rests on, in the suite rather than only in the
    // issue: a switch commanded on is a plain `r_on` resistance, so `(Va - Vb) / r_on` is its
    // current exactly. Checked against a zero-volt sense source in series with it.
    let netlist = write(
        "i_switch.cir",
        "VIN in 0 12
DSW in mid idealswitchmodel
DSW kind=ideal_switch r_on=0.01 body_g_breakdown=0 body_v_breakdown=-100 body_g_off=0 body_v_f=0.5 body_g_on=100 gate=block ctrl=GATE
VS1 mid sw 0
D1 0 sw idealdiodemodel
D1 kind=ideal_diode g_breakdown=0 v_breakdown=-100 g_off=0 v_f=0.4 g_on=100
L1 sw out 1e-3
C1 out 0 1e-4
R1 out 0 5
DUTY kind=const value=0.4
PWM1 kind=pwm freq=50000 in=DUTY
GATE kind=sig2phys domain=voltage in=PWM1
",
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
    let csv = String::from_utf8_lossy(&output.stdout);
    let mut lines = csv.lines();
    let header: Vec<&str> = lines.next().expect("a header").split(',').collect();
    let column = |name: &str| -> usize {
        header
            .iter()
            .position(|h| *h == name)
            .unwrap_or_else(|| panic!("no column {name} in {header:?}"))
    };
    let (v_in, v_mid, i_sense, gate) = (
        column("V(in)"),
        column("V(mid)"),
        column("I(VS1)"),
        column("GATE"),
    );

    let mut worst = 0.0_f64;
    let mut checked = 0usize;
    let mut off_rows = 0usize;
    for line in lines {
        let f: Vec<f64> = line
            .split(',')
            .map(|v| v.parse().unwrap_or(f64::NAN))
            .collect();
        let computed = (f[v_in] - f[v_mid]) / 0.01;
        if f[gate] >= 0.5 {
            let measured = f[i_sense];
            worst = worst.max((computed - measured).abs() / measured.abs().max(1e-12));
            checked += 1;
        } else {
            // While commanded off the same expression is nonsense -- the full blocking voltage
            // over `r_on`. Counted so the test proves it exercised both states, which is what
            // makes the gating in `conduction_power` necessary rather than decorative.
            assert!(
                computed.abs() > 100.0 && f[i_sense].abs() < 1e-9,
                "off-state row should show a true 0 A and an absurd computed current: {line}"
            );
            off_rows += 1;
        }
    }
    assert!(
        checked > 100 && off_rows > 100,
        "{checked} on, {off_rows} off"
    );
    assert!(
        worst < 1e-7,
        "computed and measured switch current should agree: worst relative error {worst:e}"
    );
}

#[test]
fn conduction_loss_follows_the_duty_split_across_a_synchronous_leg() {
    // The sharp physical claim: at D = 0.4 the low-side switch conducts for 60% of the period
    // and the high-side for 40%, carrying the same inductor current, so their conduction losses
    // must be in the ratio (1-D)/D = 1.5. That ratio is what tests the attribution -- an
    // absolute figure could be right for both switches while being assigned to the wrong one.
    let netlist = write(
        "cond_split.cir",
        &format!(
            "{SYNC_BUCK_CHANNELS}\
             LOSS_H kind=rule type=per_period target=P_cond(DSH) measure=mean period=2e-3 max=0 severity=info\n\
             LOSS_L kind=rule type=per_period target=P_cond(DSL) measure=mean period=2e-3 max=0 severity=info\n"
        ),
    );
    run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "6e-3",
        "--dt",
        "1e-7",
        "--out-every",
        "2000000",
    ]);
    let text = fs::read_to_string(netlist.with_extension("events.txt")).expect("the text log");
    let last = |rule: &str| -> f64 {
        text.lines()
            .rfind(|l| l.contains(rule) && l.contains(" at "))
            .and_then(|l| {
                let after = l.split(" at ").nth(1)?;
                after.split_whitespace().next()?.parse().ok()
            })
            .unwrap_or_else(|| panic!("no finding for {rule} in:\n{text}"))
    };
    let (high, low) = (last("P_cond(DSH)"), last("P_cond(DSL)"));
    // Measured 0.0069 W and 0.0105 W against the hand estimate (I_out = 0.96 A):
    // 0.4 * 0.96^2 * 0.02 = 0.00737 W and 0.6 * 0.96^2 * 0.02 = 0.01106 W -- the deck's dead
    // time and its conduction drops put the real figures a few percent below the ideal-duty
    // estimate, which is why the band here is loose and the ratio below is not.
    assert!(
        (0.005..0.009).contains(&high) && (0.008..0.013).contains(&low),
        "losses out of the expected range: high {high} W, low {low} W"
    );
    let ratio = low / high;
    assert!(
        (ratio - 1.5).abs() < 0.1,
        "the low-side should carry (1-D)/D = 1.5 times the high-side's conduction loss, got {ratio}"
    );
}

#[test]
fn a_switch_that_is_never_commanded_on_reports_no_conduction_loss() {
    // The failure mode the gating prevents, end to end: a permanently-off switch stands off the
    // full bus, so an ungated `(Va - Vb)^2 / r_on` would report 7.2 kW of loss in a device that
    // dissipates nothing.
    let netlist = write(
        "cond_off.cir",
        "VIN in 0 12
DSW in mid idealswitchmodel
DSW kind=ideal_switch r_on=0.02 body_g_breakdown=0 body_v_breakdown=-100 body_g_off=0 body_v_f=0.5 body_g_on=100 gate=block ctrl=GATE
R1 mid 0 1e6
OFF kind=const value=0
GATE kind=sig2phys domain=voltage in=OFF
NO_LOSS kind=rule type=per_period target=P_cond(DSW) measure=mean period=1e-5 max=1e-9 severity=error
",
    );
    let output = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "1e-4",
        "--dt",
        "1e-6",
        "--out-every",
        "100000",
    ]);
    let text = fs::read_to_string(netlist.with_extension("events.txt")).expect("the text log");
    assert!(
        text.contains("P_cond(DSW).NO_LOSS") && text.contains("passed"),
        "a switch that never turns on dissipates nothing in its channel:\n{text}"
    );
    assert_eq!(output.status.code(), Some(0), "{text}");
}

// ---------------------------------------------------------------------------
// Was it really ZVS?
// ---------------------------------------------------------------------------

#[test]
fn a_synchronous_leg_reports_its_hard_switching_edge_and_passes_its_soft_one() {
    // One deck shows both answers, which is what makes it a test rather than a demonstration.
    //
    // At D = 0.4 with dead time, the two switches of the same leg commutate differently: DSH
    // turns on into the full bus, because nothing has discharged the node; DSL turns on *behind
    // its own body diode*, which has been freewheeling through the dead time, so the voltage
    // across it is already a diode drop. That is textbook ZVS, and a rule that cannot tell the
    // two apart is useless.
    let netlist = write(
        "zvs_leg.cir",
        &format!(
            "{SYNC_BUCK_CHANNELS}\
             ZVS_H kind=rule type=zvs switches=DSH v_max=5 severity=error\n\
             ZVS_L kind=rule type=zvs switches=DSL v_max=5 severity=error\n"
        ),
    );
    run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "2e-4",
        "--dt",
        "1e-8",
        "--out-every",
        "1000000",
    ]);
    let text = fs::read_to_string(netlist.with_extension("events.txt")).expect("the text log");
    assert!(
        text.contains("ERROR DSH.ZVS_H") && text.contains("DSH turned on with V(in,mid)"),
        "the high-side switch hard-switches and should be reported:\n{text}"
    );
    // ~12.5 V: the 12 V bus plus the freewheeling body diode's forward drop.
    assert!(
        text.contains("at     12.5"),
        "the reported voltage should be the bus it switched into:\n{text}"
    );
    assert!(
        text.contains("DSL.ZVS_L") && text.contains("passed"),
        "the low-side switch commutates behind its body diode and should pass:\n{text}"
    );
}

#[test]
fn the_low_sides_pass_is_a_measured_diode_drop_and_not_a_skipped_measurement() {
    // The companion to the test above, and the reason it is a separate one: "passed" could mean
    // "measured and small" or "never measured at all", and only tightening the bound
    // distinguishes them. At v_max = 10 mV the same edge reports 0.508 V -- the body diode's
    // own drop, v_f = 0.5 plus i/g_on = 0.96/100, which agrees to a milli-volt.
    let netlist = write(
        "zvs_tight.cir",
        &format!(
            "{SYNC_BUCK_CHANNELS}\
             ZVS_L kind=rule type=zvs switches=DSL v_max=0.01 severity=error\n"
        ),
    );
    run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "2e-4",
        "--dt",
        "1e-8",
        "--out-every",
        "1000000",
    ]);
    let text = fs::read_to_string(netlist.with_extension("events.txt")).expect("the text log");
    assert!(
        text.contains("ERROR DSL.ZVS_L") && text.contains("at     0.50"),
        "the low-side's turn-on voltage is its body diode's drop, and it is measured:\n{text}"
    );
}

#[test]
fn the_reported_voltage_is_the_one_before_the_transition_not_after_it() {
    // The bug a first version shipped, kept as a test. At the step where the gate *becomes* on,
    // the switch has already been stamped as an r_on resistance and the voltage across it has
    // collapsed to i*r_on -- so reading it there reported a 400 V hard-switched leg as passing.
    // A 400 V deck with v_max = 5 therefore distinguishes the two implementations sharply: the
    // correct one reports ~400 V, the wrong one reports nothing at all.
    let netlist = write(
        "zvs_400.cir",
        "VIN in 0 400
DSW in mid idealswitchmodel
DSW kind=ideal_switch r_on=0.01 body_g_breakdown=0 body_v_breakdown=-1000 body_g_off=0 body_v_f=0.5 body_g_on=100 gate=block ctrl=GATE
L1 mid out 1e-3
R1 out 0 40
D1 0 mid idealdiodemodel
D1 kind=ideal_diode g_breakdown=0 v_breakdown=-1000 g_off=0 v_f=0.4 g_on=100
DUTY kind=const value=0.5
PWM1 kind=pwm freq=20000 in=DUTY
GATE kind=sig2phys domain=voltage in=PWM1
ZVS kind=rule type=zvs switches=DSW v_max=5 severity=error
",
    );
    run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "6e-5",
        "--dt",
        "1e-8",
        "--out-every",
        "1000000",
    ]);
    let text = fs::read_to_string(netlist.with_extension("events.txt")).expect("the text log");
    assert!(
        text.contains("ERROR DSW.ZVS") && text.contains("at     400."),
        "the blocking voltage, not the post-commutation drop:\n{text}"
    );
}

// ---------------------------------------------------------------------------
// The device-parameter vocabulary
// ---------------------------------------------------------------------------

#[test]
fn a_gate_threshold_above_the_signal_keeps_the_switch_off_through_the_whole_run() {
    // `v_gs_th` has to *do* something end to end, or it is a second trap rather than the cure
    // for the first one. A logic-level gate signal reaching 1 cannot turn on a switch whose
    // threshold is 5, so the switch never commutates -- which `type=switch_timing` reports.
    let deck = |v_gs_th: &str| {
        format!(
            "VIN in 0 12
DSW in mid idealswitchmodel
DSW kind=ideal_switch r_on=0.02 body_g_breakdown=0 body_v_breakdown=-100 body_g_off=0 \
body_v_f=0.5 body_g_on=100 gate=block ctrl=GATE{v_gs_th}
R1 mid 0 5
DUTY kind=const value=0.5
PWM1 kind=pwm freq=50000 in=DUTY
GATE kind=sig2phys domain=voltage in=PWM1
TIMING kind=rule type=switch_timing switches=DSW f_min=1000 f_max=100000 severity=error
"
        )
    };
    // Default threshold: the switch commutates at 50 kHz and the rule passes.
    let normal = write("vgsth_default.cir", &deck(""));
    run(&[
        normal.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "5e-4",
        "--dt",
        "1e-7",
        "--out-every",
        "100000",
    ]);
    let text = fs::read_to_string(normal.with_extension("events.txt")).expect("the text log");
    assert!(
        text.contains("DSW.TIMING") && text.contains("passed"),
        "with the default threshold the switch should commutate:\n{text}"
    );

    // A 5 V threshold against a logic-level signal: never on.
    let high = write("vgsth_five.cir", &deck(" v_gs_th=5"));
    run(&[
        high.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "5e-4",
        "--dt",
        "1e-7",
        "--out-every",
        "100000",
    ]);
    let text = fs::read_to_string(high.with_extension("events.txt")).expect("the text log");
    assert!(
        text.contains("ERROR DSW.TIMING"),
        "a 5 V gate threshold must keep a logic-level-driven switch off:\n{text}"
    );
}

#[test]
fn the_old_device_field_names_are_refused_with_the_replacement_named() {
    // Hard errors rather than accepted aliases: an alias is how a double meaning survives a
    // rename, and this one silently changed how a converter freewheeled.
    let cases: [(&str, &str, &str); 3] = [
        // (what the deck writes, the field refused, what the message must offer)
        ("body_v_f=0.5", "v_th=0.5", "v_gs_th"),
        ("body_g_on=100", "g_on=100", "body_g_on"),
        (
            "body_v_breakdown=-100",
            "v_breakdown=-100",
            "body_v_breakdown",
        ),
    ];
    for (correct, old, expected) in cases {
        let deck = "VIN in 0 12
DSW in mid idealswitchmodel
DSW kind=ideal_switch r_on=0.02 body_g_breakdown=0 body_v_breakdown=-100 body_g_off=0 \
body_v_f=0.5 body_g_on=100 gate=block ctrl=GATE
R1 mid 0 5
ONE kind=const value=1
GATE kind=sig2phys domain=voltage in=ONE
"
        .replace(correct, old);
        let netlist = write("old_vocab.cir", &deck);
        let output = run(&[netlist.to_str().unwrap(), "--mode", "dc"]);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("no longer exists") && stderr.contains(expected),
            "writing '{old}' should be refused and should name '{expected}':\n{stderr}"
        );
        assert_ne!(output.status.code(), Some(0), "{stderr}");
    }
}

// ---------------------------------------------------------------------------
// A deck as its own regression test
// ---------------------------------------------------------------------------

#[test]
fn an_expectation_decides_the_exit_code_so_a_deck_is_its_own_regression_test() {
    // Phase 6 asked for "expected-result assertions so a circuit carries its own regression
    // test". No new rule type was needed -- an ordinary `type=outside` with `after=` is the
    // assertion, and the exit code is what makes it usable from CI with no wrapper script.
    //
    // That property is the whole feature, so it is pinned here rather than left incidental:
    // the same deck, differing only in whether the expectation is true, must exit 0 and 2.
    let deck = |low: &str, high: &str| {
        format!(
            "VIN in 0 12
DSW in sw idealswitchmodel
DSW kind=ideal_switch r_on=0.01 body_g_breakdown=0 body_v_breakdown=-100 body_g_off=0 \
body_v_f=0.5 body_g_on=100 gate=block ctrl=GATE
D1 0 sw idealdiodemodel
D1 kind=ideal_diode g_breakdown=0 v_breakdown=-100 g_off=0 v_f=0.4 g_on=100
L1 sw out 1e-3
C1 out 0 1e-4
R1 out 0 5
DUTY kind=const value=0.4
PWM1 kind=pwm freq=50000 in=DUTY
GATE kind=sig2phys domain=voltage in=PWM1
EXPECT kind=rule type=outside target=V(out) min={low} max={high} after=3e-3 severity=error
"
        )
    };
    let at = |name: &str, low: &str, high: &str| {
        let netlist = write(name, &deck(low, high));
        let output = run(&[
            netlist.to_str().unwrap(),
            "--mode",
            "transient",
            "--tfinal",
            "5e-3",
            "--dt",
            "2e-7",
            "--out-every",
            "1000000",
        ]);
        let log = fs::read_to_string(netlist.with_extension("events.txt")).expect("the text log");
        (output.status.code(), log)
    };

    // The buck settles to about 4.8 V, so [4, 5] is a true expectation.
    let (code, log) = at("expect_true.cir", "4.0", "5.0");
    assert_eq!(code, Some(0), "a met expectation must exit 0:\n{log}");
    assert!(
        log.contains("V(out).EXPECT") && log.contains("passed"),
        "{log}"
    );

    // The same deck with an expectation it cannot meet.
    let (code, log) = at("expect_false.cir", "9.0", "10.0");
    assert_eq!(code, Some(2), "a failed expectation must exit 2:\n{log}");
    // And it has to say what it actually saw, or the CI failure is unactionable.
    assert!(
        log.contains("ERROR V(out).EXPECT") && log.contains("worst"),
        "the report should carry the worst value and when it happened:\n{log}"
    );
}

// ---------------------------------------------------------------------------
// Does the loss model describe the circuit at all?
// ---------------------------------------------------------------------------

/// A buck with ports on both sides and both of its conducting devices nameable.
///
/// `r_on=0.05` and a 0.4 V diode are chosen so the two losses are far apart: measured over a
/// settled 2 ms window the diode dissipates 0.2227 W and the switch 0.0165 W, so the diode is
/// 93% of the conduction loss. A deck where they were comparable would not show whether the
/// check actually notices a missing device.
const BUCK_FOR_CLOSURE: &str = "\
VIN in 0 12
DSW in sw idealswitchmodel
DSW kind=ideal_switch r_on=0.05 body_g_breakdown=0 body_v_breakdown=-100 body_g_off=0 body_v_f=0.5 body_g_on=100 gate=block ctrl=GATE
D1 0 sw idealdiodemodel
D1 kind=ideal_diode g_breakdown=0 v_breakdown=-100 g_off=0 v_f=0.4 g_on=100
L1 sw out 1e-3
C1 out 0 1e-4
VSENSE out load 0
R1 load 0 5
DUTY kind=const value=0.4
PWM1 kind=pwm freq=50000 in=DUTY
GATE kind=sig2phys domain=voltage in=PWM1
P_IN kind=port v=(in,0) i=-VIN
P_OUT kind=port v=(out,0) i=VSENSE
";

fn closure_run(name: &str, card: &str) -> String {
    let netlist = write(name, &format!("{BUCK_FOR_CLOSURE}{card}"));
    run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "1e-2",
        "--dt",
        "1e-7",
        "--out-every",
        "2000000",
    ]);
    fs::read_to_string(netlist.with_extension("events.txt")).expect("the text log")
}

#[test]
fn a_fully_modelled_converter_closes_its_loss_budget() {
    // Cross-checked against an independent calculation rather than trusted: integrating the
    // same CSV in Python with a plain trapezoid gave a 0.2433 W deficit against 0.2391 W of
    // modelled loss, so 0.0969% unaccounted. The engine, which accumulates with sub-step
    // interpolation through a different code path, reports 9.6869e-4 over the same window.
    // Four significant figures from two independent computations.
    let text = closure_run(
        "closure_good.cir",
        "CLOSE kind=rule type=loss_closure in=P_IN out=P_OUT devices=DSW,D1 max=0.002 \
         period=2e-3 after=6e-3 severity=error\n",
    );
    assert!(
        text.contains("P_OUT/P_IN.CLOSE") && text.contains("passed"),
        "a buck whose only losses are conduction should close to better than 0.2%:\n{text}"
    );
}

#[test]
fn omitting_the_freewheeling_diode_shows_up_as_unaccounted_power() {
    // The check's reason for existing. Every per-device figure can be individually correct
    // while the model as a whole describes almost none of the loss -- here the switch's
    // 0.0165 W is a true number about the switch and 7% of the real total.
    let text = closure_run(
        "closure_missing.cir",
        "CLOSE kind=rule type=loss_closure in=P_IN out=P_OUT devices=DSW max=0.002 \
         period=2e-3 after=6e-3 severity=error\n",
    );
    assert!(
        text.contains("ERROR P_OUT/P_IN.CLOSE"),
        "a loss model missing the dominant device should be reported:\n{text}"
    );
    // The finding has to carry the deficit *and* what the model accounted for, because the
    // difference between the two is the only thing that tells a reader where to look.
    assert!(
        text.contains("deficit, model accounts for"),
        "the finding should show both sides of the comparison:\n{text}"
    );
}

#[test]
fn the_unaccounted_fraction_is_far_larger_with_the_diode_left_out() {
    // A bound either side of both answers would be arbitrary; what matters is that the two
    // cases are separated by a wide margin rather than by a hair. Measured: 0.0969% with both
    // devices, 5.62% with the switch alone -- a factor of 58.
    let with_both = closure_run(
        "closure_cmp_both.cir",
        "CLOSE kind=rule type=loss_closure in=P_IN out=P_OUT devices=DSW,D1 max=0.0 \
         period=2e-3 after=6e-3 severity=info\n",
    );
    let switch_only = closure_run(
        "closure_cmp_one.cir",
        "CLOSE kind=rule type=loss_closure in=P_IN out=P_OUT devices=DSW max=0.0 \
         period=2e-3 after=6e-3 severity=info\n",
    );
    let fraction = |text: &str| -> f64 {
        text.lines()
            .rfind(|l| l.contains("P_OUT/P_IN.CLOSE") && l.contains(" at "))
            .and_then(|l| {
                l.split(" at ")
                    .nth(1)?
                    .split_whitespace()
                    .next()?
                    .parse()
                    .ok()
            })
            .unwrap_or_else(|| panic!("no finding in:\n{text}"))
    };
    let (both, one) = (fraction(&with_both), fraction(&switch_only));
    assert!(
        both < 0.002,
        "with both devices modelled the residual should be small, got {both}"
    );
    assert!(
        one > 20.0 * both,
        "leaving out 93% of the loss should be unmistakable: {one} vs {both}"
    );
}

/// `BUCK` with one rule that fires early, for the tests about *where* the event files go rather
/// than what is in them.
const BUCK_WITH_RATING: &str = "\
VIN vin 0 12
DSW vin sw idealswitchmodel
DSW kind=ideal_switch r_on=0.05 body_g_breakdown=0 body_v_breakdown=-100 body_g_off=0 body_v_f=0.5 body_g_on=5 gate=block ctrl=GATE
D1 0 sw dmodel
D1 kind=ideal_diode g_breakdown=0 v_breakdown=-100 g_off=0 v_f=0.4 g_on=5
L1 sw out 22e-6
C1 out 0 47e-6 ic=3
RL out 0 4
VOUT kind=phys2sig node=out
REF kind=const value=5
ERR kind=sum inputs=REF,VOUT signs=1,-1
PI kind=pid in=ERR kp=0.02 ki=50 kd=0 n=100 clamp_lo=0.05 clamp_hi=0.95 ic=0.4
PWM1 kind=pwm freq=100000 in=PI
GATE kind=sig2phys domain=voltage in=PWM1
R_vout kind=rule type=above target=V(out) max=1 unit=V severity=warning action=console
";

// --- Where the event stream goes -------------------------------------------------------------
//
// The bug these exist for: both files were derived from the netlist path with no way to ask for
// anything else, so the GUI -- which compiles a document in memory and writes the `.cir` into a
// temporary directory -- had both of them created inside that directory and deleted with it.
// Every event of every run was discarded, and no consumer could have found them without
// reconstructing the `<stem>.events.*` convention from outside.

#[test]
fn the_event_stream_goes_where_it_is_asked_for() {
    let netlist = write("redirected.cir", BUCK_WITH_RATING);
    let jsonl = scratch("asked-for.jsonl");
    let text = scratch("asked-for.txt");
    let _ = fs::remove_file(&jsonl);
    let _ = fs::remove_file(&text);
    let _ = fs::remove_file(netlist.with_extension("events.jsonl"));
    let _ = fs::remove_file(netlist.with_extension("events.txt"));

    let out = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "2e-4",
        "--dt",
        "1e-7",
        "--events",
        jsonl.to_str().unwrap(),
        "--events-text",
        text.to_str().unwrap(),
    ]);

    let log = fs::read_to_string(&jsonl).expect("the redirected JSONL log");
    assert!(log.contains("\"rule\":"), "{log}");
    assert!(fs::read_to_string(&text).is_ok(), "the redirected text log");
    // And nothing was left beside the netlist, which is the whole point: a caller that asked
    // for a path does not also get a file somewhere it never looks.
    assert!(
        !netlist.with_extension("events.jsonl").exists(),
        "a log was still written beside the netlist"
    );
    assert!(!netlist.with_extension("events.txt").exists());
    assert!(
        out.status.code() == Some(0) || out.status.code() == Some(2),
        "{:?}: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn either_file_moves_without_the_other() {
    // A tool reads the JSONL and a person reads the text. Forcing them to move together hands
    // one of the two a file nobody opens.
    let netlist = write("one-moved.cir", BUCK_WITH_RATING);
    let jsonl = scratch("only-this-one.jsonl");
    let _ = fs::remove_file(&jsonl);
    let _ = fs::remove_file(netlist.with_extension("events.txt"));

    run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "2e-4",
        "--dt",
        "1e-7",
        "--events",
        jsonl.to_str().unwrap(),
    ]);

    assert!(fs::read_to_string(&jsonl).is_ok(), "the redirected JSONL");
    assert!(
        netlist.with_extension("events.txt").exists(),
        "the text log kept its default path"
    );
    assert!(!netlist.with_extension("events.jsonl").exists());
}

#[test]
fn without_the_flags_the_files_keep_their_old_paths() {
    let netlist = write("default-paths.cir", BUCK_WITH_RATING);
    let _ = fs::remove_file(netlist.with_extension("events.jsonl"));
    let _ = fs::remove_file(netlist.with_extension("events.txt"));

    run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "2e-4",
        "--dt",
        "1e-7",
    ]);

    assert!(netlist.with_extension("events.jsonl").exists());
    assert!(netlist.with_extension("events.txt").exists());
}

#[test]
fn a_log_path_that_cannot_be_created_is_an_error_and_not_a_silently_skipped_log() {
    // Losing the diagnostics channel without a word is the failure these flags exist to fix, so
    // failing to open one is fatal and names the path.
    let netlist = write("bad-log-path.cir", BUCK_WITH_RATING);
    let missing = scratch("no-such-directory").join("run.jsonl");

    let out = run(&[
        netlist.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "2e-4",
        "--dt",
        "1e-7",
        "--events",
        missing.to_str().unwrap(),
    ]);

    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("creating the event log"), "{stderr}");
    assert!(stderr.contains("run.jsonl"), "{stderr}");
}
