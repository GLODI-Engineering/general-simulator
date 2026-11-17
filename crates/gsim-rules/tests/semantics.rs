//! The behaviour the spec is specific about, pinned.
//!
//! These are the properties a reader of the log depends on without thinking about them: that a
//! long violation is two lines, that the time printed is where it crossed, that a hold time
//! silences a numerical spike without silencing a real event, and that a rule which could not
//! read its quantity says so instead of passing.

use std::collections::BTreeMap;

use gsim_rules::{Action, Condition, EventKind, Rule, RuleEngine, Sample, Severity, Watch};

/// A sample that carries scalars and switch states, as a solver step does.
#[derive(Default)]
struct Step {
    values: BTreeMap<String, f64>,
    switches: BTreeMap<String, bool>,
}

impl Sample for Step {
    fn get(&self, quantity: &str) -> Option<f64> {
        self.values.get(quantity).copied()
    }
    fn switch_on(&self, switch: &str) -> Option<bool> {
        self.switches.get(switch).copied()
    }
}

fn scalar(name: &str, value: f64) -> Step {
    Step {
        values: BTreeMap::from([(name.to_string(), value)]),
        switches: BTreeMap::new(),
    }
}

fn over(limit: f64, action: Action) -> Rule {
    Rule::new(
        "i_max",
        "L1",
        "i",
        Condition::Above { limit },
        Severity::Warning,
        action,
    )
}

#[test]
fn a_long_violation_is_two_records_not_one_per_step() {
    // The spec's own figure: 3 ms at 10 ns steps would be 300 000 lines if rules logged per
    // sample. This is the property the whole edge design exists for.
    let mut engine = RuleEngine::new(vec![over(50.0, Action::Log)], false);
    let dt = 10e-9;
    let mut records = Vec::new();
    for step in 0..=300_010usize {
        let t = step as f64 * dt;
        // Violating from 1 us for 3 ms, then back under.
        let violating = (1e-6..=3.001e-3).contains(&t);
        let value = if violating { 60.0 } else { 10.0 };
        records.extend(engine.observe(step, t, &scalar("i", value)));
    }
    let (tail, summaries) = engine.finish(300_010);
    records.extend(tail);

    assert_eq!(
        records.len(),
        2,
        "expected an enter and a leave, got {records:#?}"
    );
    assert_eq!(records[0].kind, EventKind::Enter);
    assert_eq!(records[1].kind, EventKind::Leave);
    assert_eq!(summaries[0].entries, 1);
    let duration = records[1].duration.expect("a leave carries its duration");
    assert!(
        (duration - 3e-3).abs() < 1e-5,
        "duration {duration} should be about 3 ms"
    );
}

#[test]
fn the_reported_time_is_where_it_crossed_inside_the_step() {
    // Rising 0 -> 100 across one 1 us step crosses 50 at the half-way point. Reporting the end
    // of the step would quantise every event to the step size, which for these circuits is the
    // same order as the dead times the rules exist to measure.
    let mut engine = RuleEngine::new(vec![over(50.0, Action::Log)], false);
    engine.observe(0, 0.0, &scalar("i", 0.0));
    let records = engine.observe(1, 1e-6, &scalar("i", 100.0));
    assert_eq!(records.len(), 1);
    let crossing = records[0].time;
    assert!(
        (crossing - 0.5e-6).abs() < 1e-12,
        "crossing reported at {crossing}, expected 0.5 us"
    );
    assert_eq!(records[0].step, 1, "the step it was noticed in");
    assert_eq!(records[0].step_time, 1e-6, "the solver's own time too");
}

#[test]
fn a_hold_time_silences_a_spike_but_counts_it() {
    let mut rule = over(50.0, Action::Log);
    rule.qualifiers.min_duration = Some(100e-9);
    let mut engine = RuleEngine::new(vec![rule], false);

    // A 20 ns excursion, the switch-model spike the spec names.
    engine.observe(0, 0.0, &scalar("i", 10.0));
    let a = engine.observe(1, 10e-9, &scalar("i", 60.0));
    let b = engine.observe(2, 30e-9, &scalar("i", 10.0));
    let (tail, summaries) = engine.finish(3);

    assert!(
        a.is_empty() && b.is_empty() && tail.is_empty(),
        "spike should be silent"
    );
    assert_eq!(summaries[0].entries, 0);
    assert_eq!(
        summaries[0].filtered, 1,
        "but it must be counted -- silence about a filter is how a hold time hides a real fault"
    );
    assert!(summaries[0].passed());
}

#[test]
fn a_violation_longer_than_the_hold_time_is_reported_from_its_own_start() {
    let mut rule = over(50.0, Action::Log);
    rule.qualifiers.min_duration = Some(100e-9);
    let mut engine = RuleEngine::new(vec![rule], false);

    engine.observe(0, 0.0, &scalar("i", 50.0));
    let mut records = Vec::new();
    for step in 1..=40usize {
        let t = step as f64 * 10e-9;
        records.extend(engine.observe(step, t, &scalar("i", 60.0)));
    }
    assert_eq!(records.len(), 1, "one enter once the hold is proven");
    // The crossing, not the moment the hold expired: the reader asks when it started.
    assert!(
        records[0].time <= 10e-9 + 1e-15,
        "enter reported at {} -- should be the crossing",
        records[0].time
    );
}

#[test]
fn hysteresis_stops_a_value_sitting_on_the_limit_from_chattering() {
    let mut rule = over(50.0, Action::Log);
    rule.qualifiers.hysteresis = Some(2.0);
    let mut engine = RuleEngine::new(vec![rule], false);

    let mut records = Vec::new();
    engine.observe(0, 0.0, &scalar("i", 40.0));
    // Oscillating either side of the limit by a hair, ten times.
    for step in 1..=20usize {
        let value = if step % 2 == 0 { 50.5 } else { 49.5 };
        records.extend(engine.observe(step, step as f64 * 1e-6, &scalar("i", value)));
    }
    let (tail, _) = engine.finish(21);
    records.extend(tail);

    // Without the band this is twenty records; with it the value never falls below 48.
    assert_eq!(
        records.len(),
        2,
        "expected one enter and one truncated leave, got {records:#?}"
    );
    assert!(records[1].truncated, "still violating when the run ended");
}

#[test]
fn a_window_keeps_start_up_out_of_the_log() {
    let mut rule = over(50.0, Action::Log);
    rule.qualifiers.after = Some(1e-3);
    let mut engine = RuleEngine::new(vec![rule], false);

    let mut records = Vec::new();
    // Violating through start-up, then quiet, then violating again inside the window.
    for step in 0..=30usize {
        let t = step as f64 * 1e-4;
        let value = if t < 0.5e-3 || t > 2e-3 { 60.0 } else { 10.0 };
        records.extend(engine.observe(step, t, &scalar("i", value)));
    }
    let (tail, _) = engine.finish(31);
    records.extend(tail);

    assert!(
        records.iter().all(|r| r.time >= 1e-3 - 1e-9),
        "nothing before the window should be reported: {records:#?}"
    );
    assert!(
        !records.is_empty(),
        "the later violation must still be caught"
    );
}

#[test]
fn max_events_caps_the_log_without_losing_the_count() {
    let mut rule = over(50.0, Action::Log);
    rule.qualifiers.max_events = Some(3);
    let mut engine = RuleEngine::new(vec![rule], false);

    let mut records = Vec::new();
    for step in 0..40usize {
        let value = if step % 2 == 0 { 60.0 } else { 10.0 };
        records.extend(engine.observe(step, step as f64 * 1e-6, &scalar("i", value)));
    }
    let (tail, summaries) = engine.finish(40);
    records.extend(tail);

    assert_eq!(records.len(), 3, "the budget caps what is written");
    assert!(
        summaries[0].entries >= 10,
        "but every violation is still counted: {}",
        summaries[0].entries
    );
    assert!(summaries[0].suppressed > 0, "and the gap is stated");
}

#[test]
fn stop_after_tolerates_the_first_and_stops_on_the_nth() {
    // The spec's case: a first bad commutation is tolerable, a hundred are not.
    let mut rule = over(50.0, Action::Stop);
    rule.qualifiers.stop_after = Some(3);
    let mut engine = RuleEngine::new(vec![rule], false);

    engine.observe(0, 0.0, &scalar("i", 10.0));
    let mut stopped_at = None;
    for step in 1..20usize {
        let value = if step % 2 == 1 { 60.0 } else { 10.0 };
        engine.observe(step, step as f64 * 1e-6, &scalar("i", value));
        if engine.stop_requested().is_some() && stopped_at.is_none() {
            stopped_at = Some(step);
        }
    }
    assert!(stopped_at.is_some(), "it must stop eventually");
    // Entries 1 and 2 are tolerated; the third asks for the stop.
    assert_eq!(stopped_at, Some(5), "stopped at step {stopped_at:?}");
}

#[test]
fn a_rule_whose_quantity_is_absent_is_not_reported_as_passed() {
    // The failure this whole feature exists to prevent, applied to itself: a rule watching a
    // signal that does not exist is silent, and silence reads as "fine".
    let mut engine = RuleEngine::new(vec![over(50.0, Action::Log)], false);
    for step in 0..100usize {
        engine.observe(step, step as f64 * 1e-6, &scalar("a_different_name", 999.0));
    }
    let (records, summaries) = engine.finish(100);
    assert!(records.is_empty());
    assert_eq!(summaries[0].missing_samples, 100);
    assert!(
        !summaries[0].passed(),
        "a rule that never read its quantity has not passed"
    );
    assert!(summaries[0].line().contains("NOT EVALUATED"));
}

/// Both switches of one leg conducting at once: the shoot-through the owner asked for.
fn leg_rule(action: Action, min_deadtime: Option<f64>) -> Rule {
    Rule {
        name: "shoot_through".into(),
        target: "leg_a".into(),
        watch: Watch::ForbiddenState {
            switches: vec!["S_ah".into(), "S_al".into()],
            max_on: 1,
            min_deadtime,
        },
        unit: " switch(es)".into(),
        condition: Condition::Above { limit: 1.0 },
        qualifiers: Default::default(),
        severity: Severity::Fatal,
        action,
    }
}

fn leg(high: bool, low: bool) -> Step {
    Step {
        values: BTreeMap::new(),
        switches: BTreeMap::from([("S_ah".to_string(), high), ("S_al".to_string(), low)]),
    }
}

#[test]
fn two_switches_of_a_leg_conducting_together_is_caught_and_names_them() {
    let mut engine = RuleEngine::new(vec![leg_rule(Action::Log, None)], false);
    let mut records = Vec::new();
    // Normal commutation: high on, both off, low on -- never both.
    records.extend(engine.observe(0, 0.0, &leg(true, false)));
    records.extend(engine.observe(1, 1e-6, &leg(false, false)));
    records.extend(engine.observe(2, 2e-6, &leg(false, true)));
    assert!(
        records.is_empty(),
        "a clean commutation must be silent: {records:#?}"
    );

    // Now both at once.
    records.extend(engine.observe(3, 3e-6, &leg(true, true)));
    records.extend(engine.observe(4, 4e-6, &leg(false, true)));
    let (tail, summaries) = engine.finish(5);
    records.extend(tail);

    assert_eq!(records.len(), 2, "an enter and a leave: {records:#?}");
    assert_eq!(records[0].kind, EventKind::Enter);
    assert_eq!(records[0].severity, Severity::Fatal);
    let detail = records[0].detail.as_deref().unwrap_or_default();
    assert!(
        detail.contains("S_ah") && detail.contains("S_al"),
        "the report must name the offending switches, got {detail:?}"
    );
    assert_eq!(summaries[0].entries, 1);
}

#[test]
fn a_leg_rule_can_stop_the_run_the_moment_the_short_appears() {
    let mut engine = RuleEngine::new(vec![leg_rule(Action::Stop, None)], false);
    engine.observe(0, 0.0, &leg(true, false));
    assert!(engine.stop_requested().is_none());
    engine.observe(1, 1e-6, &leg(true, true));
    // `<target>.<name>`, the label every report prints. The target is part of it because a
    // rule's name is not always unique on its own: a rating written on a device line is named
    // for its field, so "stopped by rule 'i_max'" would not say which component.
    assert_eq!(engine.stop_requested(), Some("leg_a.shoot_through"));
}

#[test]
fn dead_time_is_measured_from_the_real_transitions() {
    // 200 ns demanded, 150 ns delivered: the gap between the high switch opening and the low
    // one closing, measured from the states rather than read from the modulator's parameter.
    let mut engine = RuleEngine::new(vec![leg_rule(Action::Log, Some(200e-9))], false);
    let mut records = Vec::new();
    records.extend(engine.observe(0, 0.0, &leg(true, false)));
    records.extend(engine.observe(1, 1000e-9, &leg(false, false)));
    records.extend(engine.observe(2, 1150e-9, &leg(false, true)));
    let (tail, summaries) = engine.finish(3);
    records.extend(tail);

    let short = records
        .iter()
        .find(|r| r.kind == EventKind::Instant)
        .unwrap_or_else(|| panic!("expected a dead-time event, got {records:#?}"));
    assert!(
        (short.value - 150e-9).abs() < 1e-15,
        "measured {}",
        short.value
    );
    assert_eq!(short.limit, 200e-9);
    assert!((summaries[0].shortest_deadtime.unwrap() - 150e-9).abs() < 1e-15);
}

#[test]
fn an_adequate_dead_time_raises_nothing_but_is_still_measured() {
    let mut engine = RuleEngine::new(vec![leg_rule(Action::Log, Some(200e-9))], false);
    engine.observe(0, 0.0, &leg(true, false));
    engine.observe(1, 1000e-9, &leg(false, false));
    engine.observe(2, 1400e-9, &leg(false, true));
    let (records, summaries) = engine.finish(3);
    assert!(
        records.is_empty(),
        "400 ns clears a 200 ns minimum: {records:#?}"
    );
    assert!((summaries[0].shortest_deadtime.unwrap() - 400e-9).abs() < 1e-15);
    assert!(summaries[0]
        .line()
        .contains("shortest dead time 400.0000 ns"));
}

#[test]
fn opening_every_switch_while_current_flows_is_caught() {
    // The IMDAB3R case: the secondary bridge must keep a path while the transformer current
    // flows, or the current has nowhere to go.
    let rule = Rule {
        name: "sec_path".into(),
        target: "bridge".into(),
        watch: Watch::RequiredPath {
            switches: vec!["Q_h".into(), "Q_l".into()],
            current: "i(Vip)".into(),
            i_min: 2.0,
        },
        unit: " A".into(),
        condition: Condition::Above { limit: 2.0 },
        qualifiers: Default::default(),
        severity: Severity::Error,
        action: Action::Log,
    };
    let mut engine = RuleEngine::new(vec![rule], false);

    let step = |h: bool, l: bool, i: f64| Step {
        values: BTreeMap::from([("i(Vip)".to_string(), i)]),
        switches: BTreeMap::from([("Q_h".to_string(), h), ("Q_l".to_string(), l)]),
    };

    let mut records = Vec::new();
    // A path exists, current flowing: fine.
    records.extend(engine.observe(0, 0.0, &step(true, false, 40.0)));
    // Everything open but almost no current: nothing to strand.
    records.extend(engine.observe(1, 1e-6, &step(false, false, 0.5)));
    assert!(records.is_empty(), "{records:#?}");

    // Everything open with 40 A flowing: the current has nowhere to go.
    records.extend(engine.observe(2, 2e-6, &step(false, false, 40.0)));
    assert_eq!(records.len(), 1, "{records:#?}");
    assert_eq!(records[0].kind, EventKind::Enter);
    assert_eq!(records[0].severity, Severity::Error);
    assert!((records[0].value - 40.0).abs() < 1e-12);
}

#[test]
fn a_group_naming_switches_the_netlist_does_not_have_is_not_silently_fine() {
    let mut engine = RuleEngine::new(vec![leg_rule(Action::Log, None)], false);
    for step in 0..50usize {
        // A sample that knows other switches, but none of this rule's.
        let sample = Step {
            values: BTreeMap::new(),
            switches: BTreeMap::from([("S_other".to_string(), true)]),
        };
        engine.observe(step, step as f64 * 1e-6, &sample);
    }
    let (_, summaries) = engine.finish(50);
    assert_eq!(summaries[0].missing_samples, 50);
    assert!(
        !summaries[0].passed(),
        "a misspelt switch name must not read as a pass"
    );
}

#[test]
fn a_handover_with_no_dead_time_at_all_is_reported() {
    // The case that found a hole in the first implementation, and the one that matters most:
    // complementary gating with no dead band hands the leg straight from one switch to the
    // other in a single step, so the all-off interval a dead time would be measured from never
    // exists. Measuring only that interval reported nothing for a dead time of zero -- silence
    // for the worst case there is.
    let mut engine = RuleEngine::new(vec![leg_rule(Action::Log, Some(200e-9))], false);
    let mut records = Vec::new();
    records.extend(engine.observe(0, 0.0, &leg(true, false)));
    records.extend(engine.observe(1, 1e-6, &leg(false, true)));
    let (tail, summaries) = engine.finish(2);
    records.extend(tail);

    let short = records
        .iter()
        .find(|r| r.kind == EventKind::Instant)
        .unwrap_or_else(|| panic!("a zero dead time must be reported, got {records:#?}"));
    assert_eq!(short.value, 0.0);
    assert_eq!(summaries[0].shortest_deadtime, Some(0.0));
    // And it is not confused with a shoot-through: the two never conduct together.
    assert_eq!(summaries[0].entries, 0);
}

#[test]
fn a_shoot_through_is_not_counted_as_a_handover() {
    // prev {A} -> now {A, B} shares a switch, so it is an overlap rather than a handover; the
    // forbidden-state rule reports it and the dead-time measurement stays out of it.
    let mut engine = RuleEngine::new(vec![leg_rule(Action::Log, Some(200e-9))], false);
    engine.observe(0, 0.0, &leg(true, false));
    engine.observe(1, 1e-6, &leg(true, true));
    let (_, summaries) = engine.finish(2);
    assert_eq!(summaries[0].entries, 1, "the overlap is the finding");
    assert_eq!(
        summaries[0].shortest_deadtime, None,
        "an overlap is not a dead time of zero -- there was no handover to measure"
    );
}

// ---------------------------------------------------------------------------------------------
// `Watch::SwitchTiming`: when a switch changes, rather than what state it is in. See #52.
// ---------------------------------------------------------------------------------------------

fn timing(min_on: Option<f64>, min_off: Option<f64>, require_commutation: bool) -> Rule {
    Rule {
        name: "T_sw".into(),
        target: "S1".into(),
        watch: Watch::SwitchTiming {
            switches: vec!["S1".into()],
            min_on,
            min_off,
            f_min: None,
            f_max: None,
            require_commutation,
        },
        unit: String::new(),
        condition: Condition::Above { limit: 0.0 },
        qualifiers: Default::default(),
        severity: Severity::Warning,
        action: Action::Log,
    }
}

fn frequency(f_min: Option<f64>, f_max: Option<f64>) -> Rule {
    Rule {
        name: "T_f".into(),
        target: "S1".into(),
        watch: Watch::SwitchTiming {
            switches: vec!["S1".into()],
            min_on: None,
            min_off: None,
            f_min,
            f_max,
            require_commutation: false,
        },
        unit: String::new(),
        condition: Condition::Above { limit: 0.0 },
        qualifiers: Default::default(),
        severity: Severity::Warning,
        action: Action::Log,
    }
}

fn switch_at(on: bool) -> Step {
    Step {
        values: BTreeMap::new(),
        switches: BTreeMap::from([("S1".to_string(), on)]),
    }
}

/// Drives one switch through `pattern`, one step per entry, `dt` apart.
fn drive(engine: &mut RuleEngine, pattern: &[bool], dt: f64) -> Vec<EventKind> {
    let mut kinds = Vec::new();
    for (step, on) in pattern.iter().enumerate() {
        for record in engine.observe(step, step as f64 * dt, &switch_at(*on)) {
            kinds.push(record.kind);
        }
    }
    kinds
}

#[test]
fn an_on_pulse_shorter_than_the_device_allows_is_reported_with_its_length() {
    let mut engine = RuleEngine::new(vec![timing(Some(3e-6), None, false)], false);
    // off, on for two steps (2 us), off again: a 2 us pulse against a 3 us minimum.
    let records: Vec<_> = {
        let mut all = Vec::new();
        for (step, on) in [false, true, true, false].iter().enumerate() {
            all.extend(engine.observe(step, step as f64 * 1e-6, &switch_at(*on)));
        }
        all
    };
    assert_eq!(records.len(), 1, "one finding, got {records:?}");
    let record = &records[0];
    assert_eq!(record.kind, EventKind::Instant, "a pulse has no interval");
    assert!((record.value - 2e-6).abs() < 1e-12, "{}", record.value);
    assert_eq!(record.limit, 3e-6);
    assert_eq!(record.unit, "s");
    let detail = record.detail.as_deref().unwrap_or_default();
    assert!(detail.contains("S1 was on for"), "{detail}");
}

#[test]
fn a_pulse_long_enough_is_not_reported() {
    // Without this the test above would pass for a check that fires on every transition.
    let mut engine = RuleEngine::new(vec![timing(Some(1e-6), None, false)], false);
    let kinds = drive(&mut engine, &[false, true, true, true, false], 1e-6);
    assert!(kinds.is_empty(), "a 3 us pulse against a 1 us minimum");
}

#[test]
fn a_short_off_interval_is_reported_too() {
    let mut engine = RuleEngine::new(vec![timing(None, Some(3e-6), false)], false);
    let records: Vec<_> = {
        let mut all = Vec::new();
        for (step, on) in [true, false, true].iter().enumerate() {
            all.extend(engine.observe(step, step as f64 * 1e-6, &switch_at(*on)));
        }
        all
    };
    assert_eq!(records.len(), 1);
    let detail = records[0].detail.as_deref().unwrap_or_default();
    assert!(detail.contains("S1 was off for"), "{detail}");
}

#[test]
fn the_interval_in_progress_at_the_first_step_is_not_judged() {
    // Its start is unknown -- it began before anything was watching -- so its length is not a
    // measurement. Reporting it would make every run that starts mid-pulse produce a finding
    // whose number means nothing.
    let mut engine = RuleEngine::new(vec![timing(Some(1.0), None, false)], false);
    let kinds = drive(&mut engine, &[true, false], 1e-6);
    assert!(
        kinds.is_empty(),
        "the first interval has no measurable start"
    );
}

#[test]
fn a_switch_that_never_commutated_is_reported_at_the_end_of_the_run() {
    // The deck whose waveform looks reasonable: the gate never reaches the switch, so the run
    // simulates a converter with a permanently open switch, which still charges and settles.
    let mut engine = RuleEngine::new(vec![timing(None, None, true)], false);
    let during = drive(&mut engine, &[false, false, false, false], 1e-6);
    assert!(during.is_empty(), "nothing is knowable until the run ends");

    let (records, summaries) = engine.finish(4);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].kind, EventKind::Instant);
    let detail = records[0].detail.as_deref().unwrap_or_default();
    assert!(detail.contains("never commutated"), "{detail}");
    assert!(detail.contains("stayed open"), "{detail}");
    assert_eq!(summaries[0].instants, 1);
    assert!(!summaries[0].passed(), "a finding is not a pass");
}

#[test]
fn a_switch_that_did_commutate_raises_nothing() {
    let mut engine = RuleEngine::new(vec![timing(None, None, true)], false);
    drive(&mut engine, &[false, true, false], 1e-6);
    let (records, summaries) = engine.finish(3);
    assert!(records.is_empty(), "{records:?}");
    assert!(summaries[0].passed());
}

#[test]
fn require_commutation_can_be_turned_off() {
    let mut engine = RuleEngine::new(vec![timing(None, None, false)], false);
    drive(&mut engine, &[false, false, false], 1e-6);
    let (records, _) = engine.finish(3);
    assert!(records.is_empty());
}

#[test]
fn a_frequency_is_measured_over_the_window_that_was_observed() {
    // Ten transitions -- five periods -- across 10 us is 500 kHz. Two transitions per period,
    // which is why this is not a count of completed cycles.
    let mut engine = RuleEngine::new(vec![frequency(None, Some(400e3))], false);
    let pattern: Vec<bool> = (0..11).map(|i| i % 2 == 1).collect();
    drive(&mut engine, &pattern, 1e-6);
    let (records, _) = engine.finish(11);
    assert_eq!(records.len(), 1, "{records:?}");
    assert!(
        (records[0].value - 500e3).abs() < 1e3,
        "measured {} Hz",
        records[0].value
    );
    assert_eq!(records[0].unit, " Hz");
    assert!(records[0]
        .detail
        .as_deref()
        .unwrap_or_default()
        .contains("above"));
}

#[test]
fn a_frequency_inside_its_band_raises_nothing() {
    let mut engine = RuleEngine::new(vec![frequency(Some(400e3), Some(600e3))], false);
    let pattern: Vec<bool> = (0..11).map(|i| i % 2 == 1).collect();
    drive(&mut engine, &pattern, 1e-6);
    let (records, _) = engine.finish(11);
    assert!(records.is_empty(), "500 kHz is inside 400-600 kHz");
}

#[test]
fn a_frequency_below_its_band_is_reported() {
    let mut engine = RuleEngine::new(vec![frequency(Some(600e3), None)], false);
    let pattern: Vec<bool> = (0..11).map(|i| i % 2 == 1).collect();
    drive(&mut engine, &pattern, 1e-6);
    let (records, _) = engine.finish(11);
    assert_eq!(records.len(), 1);
    assert!(records[0]
        .detail
        .as_deref()
        .unwrap_or_default()
        .contains("below"));
}

#[test]
fn a_timing_rule_whose_switch_does_not_exist_says_so_rather_than_passing() {
    // The same discipline the quantity rules follow: a rule that could not look is not a rule
    // that found nothing.
    let mut engine = RuleEngine::new(vec![timing(Some(1e-6), None, true)], false);
    for step in 0..3 {
        engine.observe(step, step as f64 * 1e-6, &scalar("i", 1.0));
    }
    let (records, summaries) = engine.finish(3);
    assert!(records.is_empty());
    assert_eq!(summaries[0].missing_samples, 3);
    assert!(!summaries[0].passed(), "unreadable is not passed");
}

// ---------------------------------------------------------------------------------------------
// Conduction checks: a slope bound, a body diode carrying current, and a commutation order.
// See #53.
// ---------------------------------------------------------------------------------------------

fn slope_rule(max_slope: f64) -> Rule {
    Rule {
        name: "R_slew".into(),
        target: "I(L1)".into(),
        watch: Watch::RateOfChange {
            quantity: "I(L1)".into(),
            max_slope,
        },
        unit: " A/s".into(),
        condition: Condition::Above { limit: max_slope },
        qualifiers: Default::default(),
        severity: Severity::Error,
        action: Action::Log,
    }
}

#[test]
fn a_slope_within_its_bound_is_not_reported() {
    let mut engine = RuleEngine::new(vec![slope_rule(1e6)], false);
    // 1 A per microsecond is 1e6 A/s exactly; stepping by 0.5 A each microsecond is half that.
    for (step, value) in [0.0, 0.5, 1.0, 1.5].iter().enumerate() {
        let records = engine.observe(step, step as f64 * 1e-6, &scalar("I(L1)", *value));
        assert!(records.is_empty(), "at step {step}: {records:?}");
    }
}

#[test]
fn a_slope_over_its_bound_is_reported_with_the_slope_it_measured() {
    let mut engine = RuleEngine::new(vec![slope_rule(1e6)], false);
    let mut all = Vec::new();
    // 3 A in one microsecond: 3e6 A/s.
    for (step, value) in [0.0, 3.0].iter().enumerate() {
        all.extend(engine.observe(step, step as f64 * 1e-6, &scalar("I(L1)", *value)));
    }
    assert_eq!(all.len(), 1, "{all:?}");
    assert!((all[0].value - 3e6).abs() < 1.0, "{}", all[0].value);
    assert_eq!(all[0].limit, 1e6);
}

#[test]
fn a_slope_is_judged_on_magnitude_so_a_collapse_counts_as_much_as_a_surge() {
    // A cut current falls as violently as it rises, and the sign of the branch depends on how
    // the element was declared. Judging the signed slope would catch half the cases.
    let mut engine = RuleEngine::new(vec![slope_rule(1e6)], false);
    let mut all = Vec::new();
    for (step, value) in [3.0, 0.0].iter().enumerate() {
        all.extend(engine.observe(step, step as f64 * 1e-6, &scalar("I(L1)", *value)));
    }
    assert_eq!(all.len(), 1, "{all:?}");
    assert!((all[0].value + 3e6).abs() < 1.0, "{}", all[0].value);
}

#[test]
fn the_first_step_of_a_slope_rule_is_not_a_missing_sample() {
    // There is no slope on the first step. Counting it as unreadable would leave every rate
    // rule marked NOT EVALUATED for a step it could never have answered, which would make the
    // summary lie about the one thing it exists to tell the truth about.
    let mut engine = RuleEngine::new(vec![slope_rule(1e6)], false);
    engine.observe(0, 0.0, &scalar("I(L1)", 5.0));
    let (_, summaries) = engine.finish(1);
    assert_eq!(summaries[0].missing_samples, 0);
    assert!(summaries[0].passed());
}

/// A sample carrying a switch's commanded state, its body-diode segment and its current.
struct Bridge {
    on: bool,
    body_diode: bool,
    current: f64,
}

impl Sample for Bridge {
    fn get(&self, quantity: &str) -> Option<f64> {
        (quantity == "I(VSL)").then_some(self.current)
    }
    fn switch_on(&self, switch: &str) -> Option<bool> {
        (switch == "DSL").then_some(self.on)
    }
    fn body_diode_on(&self, switch: &str) -> Option<bool> {
        (switch == "DSL").then_some(self.body_diode)
    }
}

fn body_diode_rule(i_min: f64) -> Rule {
    Rule {
        name: "B_dsl".into(),
        target: "DSL".into(),
        watch: Watch::BodyDiode {
            switches: vec!["DSL".into()],
            i_min,
            currents: vec!["I(VSL)".into()],
        },
        unit: " A".into(),
        condition: Condition::Above { limit: i_min },
        qualifiers: Default::default(),
        severity: Severity::Warning,
        action: Action::Log,
    }
}

#[test]
fn a_body_diode_carrying_more_than_the_threshold_is_reported() {
    let mut engine = RuleEngine::new(vec![body_diode_rule(0.5)], false);
    let records = engine.observe(
        0,
        0.0,
        &Bridge {
            on: false,
            body_diode: true,
            current: 1.3,
        },
    );
    assert_eq!(records.len(), 1, "{records:?}");
    let detail = records[0].detail.as_deref().unwrap_or_default();
    assert!(detail.contains("body diode conducting in DSL"), "{detail}");
    assert!((records[0].value - 1.3).abs() < 1e-12);
}

#[test]
fn ordinary_dead_time_freewheeling_stays_below_the_threshold_and_stays_quiet() {
    // The reason the rule has a threshold at all: a body diode carrying current is how every
    // unsynchronised dead time is survived, and reporting each one would bury the case where
    // the channel was never turned on.
    let mut engine = RuleEngine::new(vec![body_diode_rule(10.0)], false);
    let records = engine.observe(
        0,
        0.0,
        &Bridge {
            on: false,
            body_diode: true,
            current: 1.3,
        },
    );
    assert!(records.is_empty());
}

#[test]
fn a_switch_that_is_commanded_on_is_not_a_body_diode_finding() {
    // A commanded-on switch is stamped as a plain resistance conducting both ways, so it has
    // no body-diode segment and the question does not apply to it.
    let mut engine = RuleEngine::new(vec![body_diode_rule(0.5)], false);
    let records = engine.observe(
        0,
        0.0,
        &Bridge {
            on: true,
            body_diode: true,
            current: 50.0,
        },
    );
    assert!(records.is_empty());
}

fn order_rule(sequence: &[&str]) -> Rule {
    Rule {
        name: "C_ord".into(),
        target: sequence.join("->"),
        watch: Watch::CommutationOrder {
            sequence: sequence.iter().map(|s| (*s).to_string()).collect(),
        },
        unit: String::new(),
        condition: Condition::Above { limit: 0.0 },
        qualifiers: Default::default(),
        severity: Severity::Error,
        action: Action::Log,
    }
}

/// One step of a three-switch bridge.
fn three(a: bool, b: bool, c: bool) -> Step {
    Step {
        values: BTreeMap::new(),
        switches: BTreeMap::from([
            ("S1".to_string(), a),
            ("S2".to_string(), b),
            ("S3".to_string(), c),
        ]),
    }
}

#[test]
fn turn_ons_in_the_stated_order_raise_nothing() {
    let mut engine = RuleEngine::new(vec![order_rule(&["S1", "S2", "S3"])], false);
    let pattern = [
        (false, false, false),
        (true, false, false),
        (false, true, false),
        (false, false, true),
        (true, false, false),
    ];
    for (step, (a, b, c)) in pattern.iter().enumerate() {
        let records = engine.observe(step, step as f64 * 1e-6, &three(*a, *b, *c));
        assert!(records.is_empty(), "at step {step}: {records:?}");
    }
}

#[test]
fn a_turn_on_out_of_order_is_reported_and_names_what_was_expected() {
    let mut engine = RuleEngine::new(vec![order_rule(&["S1", "S2", "S3"])], false);
    let mut all = Vec::new();
    // S1, then S3 where S2 was due.
    for (step, (a, b, c)) in [
        (false, false, false),
        (true, false, false),
        (false, false, true),
    ]
    .iter()
    .enumerate()
    {
        all.extend(engine.observe(step, step as f64 * 1e-6, &three(*a, *b, *c)));
    }
    assert_eq!(all.len(), 1, "{all:?}");
    let detail = all[0].detail.as_deref().unwrap_or_default();
    assert!(detail.contains("S3 turned on out of order"), "{detail}");
    assert!(detail.contains("S2 was expected next"), "{detail}");
}

#[test]
fn a_run_that_starts_midway_through_a_sequence_is_not_reported_for_it() {
    // The phase comes from the first turn-on observed. Assuming the sequence begins at its
    // first entry would report an artefact of where the window opened.
    let mut engine = RuleEngine::new(vec![order_rule(&["S1", "S2", "S3"])], false);
    let mut all = Vec::new();
    for (step, (a, b, c)) in [
        (false, false, false),
        (false, true, false),
        (false, false, true),
        (true, false, false),
    ]
    .iter()
    .enumerate()
    {
        all.extend(engine.observe(step, step as f64 * 1e-6, &three(*a, *b, *c)));
    }
    assert!(all.is_empty(), "started at S2, which is allowed: {all:?}");
}

#[test]
fn one_departure_from_the_order_does_not_report_on_every_later_transition() {
    // Resynchronising to what happened rather than to what was wanted. Otherwise a single
    // mistake produces a finding per transition for the rest of the run, and a log with a
    // thousand lines hides the one that mattered.
    let mut engine = RuleEngine::new(vec![order_rule(&["S1", "S2", "S3"])], false);
    let mut all = Vec::new();
    for (step, (a, b, c)) in [
        (false, false, false),
        (true, false, false),
        (false, false, true), // out of order
        (true, false, false), // S1 again: correct after S3
        (false, true, false), // S2: correct after S1
    ]
    .iter()
    .enumerate()
    {
        all.extend(engine.observe(step, step as f64 * 1e-6, &three(*a, *b, *c)));
    }
    assert_eq!(all.len(), 1, "exactly one finding: {all:?}");
}

#[test]
fn a_rule_watching_a_nan_does_not_report_it_as_passed() {
    // The premise the finite check exists for, pinned here so it cannot regress quietly:
    // `NaN > limit` is false in Rust, so a rule comparing one finds no violation. Reporting
    // `passed` for that would be the single most misleading line the summary can print --
    // the question was asked, the answer was unobtainable, and the table would say the
    // circuit is fine.
    let mut engine = RuleEngine::new(vec![over(10.0, Action::Log)], false);
    let records = engine.observe(0, 0.0, &scalar("i", f64::NAN));
    assert!(
        records.is_empty(),
        "a NaN cannot cross a limit: {records:?}"
    );

    let (_, summaries) = engine.finish(1);
    assert!(
        !summaries[0].passed(),
        "a rule that only ever saw a NaN must not report passed"
    );
    assert_eq!(
        summaries[0].missing_samples, 1,
        "it is an unreadable sample"
    );
}
