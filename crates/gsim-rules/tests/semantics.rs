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
