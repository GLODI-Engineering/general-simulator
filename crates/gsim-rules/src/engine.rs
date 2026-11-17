//! The evaluator: a stream of samples in, edge events out.
//!
//! One state machine per rule, fed once per solver step. Everything it does exists to keep the
//! log readable by a person:
//!
//! * **Edges, not samples.** A violation that lasts 3 ms at 10 ns steps is two records, not
//!   300 000. Nothing here can emit per step.
//! * **Sub-step times.** The reported time is where the quantity crossed the limit between two
//!   samples, found by linear interpolation. At a 10 ns step every time would otherwise be
//!   quantised to 10 ns, which is the same order as the dead times these rules measure.
//! * **The quantity it could not read is reported.** A rule naming a signal that does not
//!   exist silently never fires, which is precisely the "plausible but wrong" outcome the whole
//!   feature exists to prevent, so a missing quantity is counted and surfaced in the summary.
//!
//! A rule spanning several components runs through the same machine: [`crate::Watch`] reduces
//! each kind to a value and a margin, so a leg conducting through both switches has the same
//! edges, hold time and duration as an over-current, and the next rule type costs a variant
//! rather than a state machine.

use std::collections::BTreeMap;

use crate::event::{Action, EventKind, EventRecord, Severity};
use crate::rule::{Rule, Watch};
use crate::summary::RuleSummary;

/// What the engine reads each step.
///
/// A trait rather than a concrete map so this crate never depends on the solver: the caller
/// adapts whatever it holds -- an operating point, a block-graph signal map -- without copying
/// every quantity into an intermediate structure each step.
pub trait Sample {
    /// The named quantity's value, or `None` when this sample does not carry it.
    fn get(&self, quantity: &str) -> Option<f64>;

    /// Whether a named switch is conducting, or `None` when this sample has no such switch.
    ///
    /// Separate from [`Sample::get`] because "is it on" is not a measurement: an ideal switch
    /// has a state the solver resolved, and asking for it as a number would invite a threshold
    /// comparison that the solver has already made correctly.
    fn switch_on(&self, _switch: &str) -> Option<bool> {
        None
    }
}

impl Sample for BTreeMap<String, f64> {
    fn get(&self, quantity: &str) -> Option<f64> {
        BTreeMap::get(self, quantity).copied()
    }
}

/// What a rule saw this step, in the one shape the state machine understands.
struct Observation {
    /// The number a report quotes.
    value: f64,
    /// Positive while violating, negative while not, zero on the line.
    margin: f64,
    /// The limit a report quotes beside the value.
    limit: f64,
    /// Which components are responsible, when the rule spans several.
    detail: Option<String>,
}

/// The live part of one violation.
#[derive(Debug, Clone)]
struct Active {
    /// The interpolated crossing time, which is what an enter reports even when the hold time
    /// delays the decision to report at all.
    started: f64,
    /// Whether the enter record has gone out: false while the hold time is still unproven.
    announced: bool,
    worst_margin: f64,
    worst_value: f64,
    worst_time: f64,
    limit: f64,
    detail: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct RuleState {
    prev_time: Option<f64>,
    prev_margin: Option<f64>,
    active: Option<Active>,
    entries: usize,
    /// Violations the hold time threw away. Counted because a large number here means the hold
    /// time is hiding something, and silence would not say so.
    filtered: usize,
    emitted: usize,
    produced: usize,
    total_time: f64,
    worst_margin: f64,
    worst_value: f64,
    worst_time: f64,
    missing: usize,
    /// Findings with no duration -- a dead time measured at a transition.
    instants: usize,
    /// For dead-time measurement: when every switch of the group last went off.
    all_off_since: Option<f64>,
    /// Which switches of the group were conducting on the previous step.
    prev_on: Vec<String>,
    /// Shortest gap measured between one switch opening and the next closing.
    shortest_deadtime: Option<f64>,
}

/// Evaluates every rule once per step and reports what changed.
pub struct RuleEngine {
    rules: Vec<Rule>,
    state: Vec<RuleState>,
    /// False for a batch run, where a `pause` has nobody to resume it and is recorded as the
    /// console line it actually became.
    interactive: bool,
    stop_requested: Option<String>,
    last_time: f64,
}

impl RuleEngine {
    pub fn new(rules: Vec<Rule>, interactive: bool) -> RuleEngine {
        let state = vec![RuleState::default(); rules.len()];
        RuleEngine {
            rules,
            state,
            interactive,
            stop_requested: None,
            last_time: 0.0,
        }
    }

    /// The rule that asked the run to end, once one has.
    pub fn stop_requested(&self) -> Option<&str> {
        self.stop_requested.as_deref()
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Feed one solver step. Returns the records it produced, which is usually none.
    pub fn observe(&mut self, step: usize, t: f64, sample: &impl Sample) -> Vec<EventRecord> {
        self.last_time = t;
        let mut out = Vec::new();
        for index in 0..self.rules.len() {
            self.observe_one(index, step, t, sample, &mut out);
        }
        out
    }

    fn observe_one(
        &mut self,
        index: usize,
        step: usize,
        t: f64,
        sample: &impl Sample,
        out: &mut Vec<EventRecord>,
    ) {
        if !self.rules[index].in_window(t) {
            // A window closing mid-violation ends the interval rather than dropping it: the
            // time already spent in violation is real and belongs in the totals.
            if let Some(active) = self.state[index].active.take() {
                self.close_interval(index, &active, t, step, t, true, out);
            }
            self.state[index].prev_margin = None;
            self.state[index].prev_time = None;
            return;
        }

        self.measure_deadtime(index, t, step, sample, out);

        let Some(observation) = self.observe_watch(index, sample) else {
            self.state[index].missing += 1;
            return;
        };
        let Observation {
            value,
            margin,
            limit,
            detail,
        } = observation;

        let leave_at = -self.rules[index].qualifiers.hysteresis.unwrap_or(0.0);
        let (prev_time, prev_margin) = (self.state[index].prev_time, self.state[index].prev_margin);

        match self.state[index].active.take() {
            None => {
                if margin > 0.0 {
                    // Where it crossed, not where we noticed.
                    let started = cross(prev_time, prev_margin, t, margin, 0.0);
                    let active = Active {
                        started,
                        announced: false,
                        worst_margin: margin,
                        worst_value: value,
                        worst_time: t,
                        limit,
                        detail,
                    };
                    // With no hold time the enter is reportable the moment it happens.
                    let active = self.announce_if_due(index, active, t, step, out);
                    self.state[index].active = Some(active);
                }
            }
            Some(mut active) => {
                if margin > active.worst_margin {
                    active.worst_margin = margin;
                    active.worst_value = value;
                    active.worst_time = t;
                    active.limit = limit;
                    active.detail = detail;
                }

                if margin <= leave_at {
                    let ended = cross(prev_time, prev_margin, t, margin, leave_at);
                    let long_enough = active.announced
                        || self.rules[index]
                            .qualifiers
                            .min_duration
                            .is_none_or(|hold| ended - active.started >= hold);
                    if long_enough {
                        if !active.announced {
                            // Short enough to have been invisible until now, long enough to
                            // count: the enter still belongs in the log, at its own time.
                            let record = enter_record(
                                &self.rules[index],
                                &active,
                                step,
                                t,
                                self.interactive,
                            );
                            self.emit(index, record, out);
                            active.announced = true;
                        }
                        self.close_interval(index, &active, ended, step, t, false, out);
                    } else {
                        self.state[index].filtered += 1;
                    }
                } else {
                    let active = self.announce_if_due(index, active, t, step, out);
                    self.state[index].active = Some(active);
                }
            }
        }

        self.state[index].prev_time = Some(t);
        self.state[index].prev_margin = Some(margin);
    }

    /// Emit the enter once the hold time is satisfied (or there is none).
    fn announce_if_due(
        &mut self,
        index: usize,
        mut active: Active,
        t: f64,
        step: usize,
        out: &mut Vec<EventRecord>,
    ) -> Active {
        if active.announced {
            return active;
        }
        let due = match self.rules[index].qualifiers.min_duration {
            None => true,
            Some(hold) => t - active.started >= hold,
        };
        if due {
            let record = enter_record(&self.rules[index], &active, step, t, self.interactive);
            self.emit(index, record, out);
            active.announced = true;
        }
        active
    }

    /// Reduce this rule's watch to a value and a margin, or `None` when the sample cannot
    /// answer it.
    fn observe_watch(&self, index: usize, sample: &impl Sample) -> Option<Observation> {
        let rule = &self.rules[index];
        match &rule.watch {
            Watch::Quantity { quantity } => {
                let value = sample.get(quantity)?;
                Some(Observation {
                    value,
                    margin: rule.condition.margin(value),
                    limit: rule.condition.reference(value),
                    detail: None,
                })
            }
            Watch::ForbiddenState {
                switches, max_on, ..
            } => {
                let on = conducting(sample, switches)?;
                let count = on.len();
                Some(Observation {
                    value: count as f64,
                    // How many more are conducting than may be: a count is interpolated like
                    // any other margin, which places the crossing inside the step that opened
                    // the short rather than at its end.
                    margin: count as f64 - *max_on as f64,
                    limit: *max_on as f64,
                    detail: (count > *max_on).then(|| on.join(", ")),
                })
            }
            Watch::RequiredPath {
                switches,
                current,
                i_min,
            } => {
                let on = conducting(sample, switches)?;
                let magnitude = sample.get(current)?.abs();
                Some(Observation {
                    value: magnitude,
                    // Violating only when the current has nowhere to go: any conducting switch
                    // is a path, so the margin is forced negative rather than computed.
                    margin: if on.is_empty() {
                        magnitude - *i_min
                    } else {
                        -(*i_min).max(1.0)
                    },
                    limit: *i_min,
                    detail: on.is_empty().then(|| switches.join(", ")),
                })
            }
        }
    }

    /// Measure the gap between the group's last switch opening and its next closing.
    ///
    /// Dead time is a property of the real transitions, not of what the modulator intended, so
    /// it is measured here from the resolved switch states rather than read from a parameter.
    fn measure_deadtime(
        &mut self,
        index: usize,
        t: f64,
        step: usize,
        sample: &impl Sample,
        out: &mut Vec<EventRecord>,
    ) {
        let Watch::ForbiddenState {
            switches,
            min_deadtime,
            ..
        } = &self.rules[index].watch
        else {
            return;
        };
        let Some(on) = conducting(sample, switches) else {
            return;
        };
        let mut measured_gap: Option<f64> = None;
        let now_on: Vec<String> = on.iter().map(|s| s.to_string()).collect();
        let conducting_now = !now_on.is_empty();
        let was_conducting = !self.state[index].prev_on.is_empty();
        // A handover straight from one switch to another, with no step in between where the
        // leg was open, is a dead time of zero -- the worst case there is. It has to be
        // detected on its own, because the all-off interval it would otherwise be measured
        // from never exists. Complementary gating with no dead band does exactly this, and
        // reporting nothing for it would be the most dangerous silence of the three.
        let handover = was_conducting
            && conducting_now
            && !self.state[index]
                .prev_on
                .iter()
                .any(|name| now_on.contains(name));
        let state = &mut self.state[index];
        if handover {
            state.shortest_deadtime = Some(0.0);
            state.all_off_since = None;
        } else if was_conducting && !conducting_now {
            state.all_off_since = Some(t);
        } else if !was_conducting && conducting_now {
            if let Some(since) = state.all_off_since.take() {
                let gap = t - since;
                if state.shortest_deadtime.is_none_or(|best| gap < best) {
                    state.shortest_deadtime = Some(gap);
                }
                measured_gap = Some(gap);
            }
        }
        self.state[index].prev_on = now_on;

        if handover {
            measured_gap = Some(0.0);
        }
        if let Some(gap) = measured_gap {
            if let Some(minimum) = *min_deadtime {
                if gap < minimum {
                    {
                        let rule = &self.rules[index];
                        let record = EventRecord {
                            time: t,
                            step,
                            step_time: t,
                            rule: rule.name.clone(),
                            target: rule.target.clone(),
                            kind: EventKind::Instant,
                            severity: rule.severity,
                            action: taken(rule.action, self.interactive),
                            value: gap,
                            limit: minimum,
                            duration: None,
                            peak: None,
                            peak_time: None,
                            truncated: false,
                            detail: Some(format!("dead time between {}", switches.join(" and "))),
                            // Seconds, not the violation's own unit: this finding is a duration.
                            unit: "s".to_string(),
                        };
                        self.state[index].instants += 1;
                        self.emit(index, record, out);
                    }
                }
            }
        }
    }

    /// Fold a finished interval into the totals and emit its leave.
    #[allow(clippy::too_many_arguments)]
    fn close_interval(
        &mut self,
        index: usize,
        active: &Active,
        ended: f64,
        step: usize,
        step_time: f64,
        truncated: bool,
        out: &mut Vec<EventRecord>,
    ) {
        let record = leave_record(
            &self.rules[index],
            active,
            ended,
            step,
            step_time,
            truncated,
            self.interactive,
        );
        // Emitted before the count is advanced so that the enter and the leave of the same
        // violation agree on which violation it is. They did not: `stop_after` counted the
        // leave as one further than the enter, and a rule told to tolerate two stopped on the
        // second.
        self.state[index].active = None;
        if active.announced {
            self.emit(index, record, out);
        }
        let state = &mut self.state[index];
        let first = state.entries == 0;
        state.entries += 1;
        state.total_time += (ended - active.started).max(0.0);
        if first || active.worst_margin > state.worst_margin {
            state.worst_margin = active.worst_margin;
            state.worst_value = active.worst_value;
            state.worst_time = active.worst_time;
        }
    }

    /// Push a record unless the rule has used up its event budget, and note a stop.
    fn emit(&mut self, index: usize, record: EventRecord, out: &mut Vec<EventRecord>) {
        let rule = &self.rules[index];
        let entries_including_this = self.state[index].entries + 1;
        let stopping = rule.action == Action::Stop
            && rule
                .qualifiers
                .stop_after
                .is_none_or(|threshold| entries_including_this >= threshold);
        if stopping && self.stop_requested.is_none() {
            self.stop_requested = Some(rule.name.clone());
        }
        let state = &mut self.state[index];
        state.produced += 1;
        if let Some(budget) = rule.qualifiers.max_events {
            if state.emitted >= budget {
                return;
            }
        }
        state.emitted += 1;
        out.push(record);
    }

    /// End the run: close anything still violating and report every rule.
    ///
    /// A violation still in progress is reported as a leave marked `truncated`, so its time
    /// counts toward the totals and the reader can see that the run ended rather than the
    /// condition.
    pub fn finish(&mut self, step: usize) -> (Vec<EventRecord>, Vec<RuleSummary>) {
        let t = self.last_time;
        let mut out = Vec::new();
        for index in 0..self.rules.len() {
            if let Some(mut active) = self.state[index].active.take() {
                if !active.announced {
                    let record =
                        enter_record(&self.rules[index], &active, step, t, self.interactive);
                    self.emit(index, record, &mut out);
                    active.announced = true;
                }
                self.close_interval(index, &active, t, step, t, true, &mut out);
            }
        }
        let summaries = self
            .rules
            .iter()
            .zip(self.state.iter())
            .map(|(rule, state)| RuleSummary {
                rule: rule.name.clone(),
                target: rule.target.clone(),
                severity: rule.severity,
                unit: rule.unit.clone(),
                entries: state.entries,
                filtered: state.filtered,
                suppressed: state.produced.saturating_sub(state.emitted),
                total_time: state.total_time,
                worst_value: (state.entries > 0).then_some(state.worst_value),
                worst_time: (state.entries > 0).then_some(state.worst_time),
                missing_samples: state.missing,
                instants: state.instants,
                shortest_deadtime: state.shortest_deadtime,
            })
            .collect();
        (out, summaries)
    }
}

/// Which of `switches` the sample says are conducting, or `None` if it knows none of them.
///
/// `None` rather than an empty list matters: a group naming switches the netlist does not have
/// must be reported as unreadable, not as a leg that is permanently open.
fn conducting<'a>(sample: &impl Sample, switches: &'a [String]) -> Option<Vec<&'a str>> {
    let mut known = false;
    let mut on = Vec::new();
    for switch in switches {
        if let Some(state) = sample.switch_on(switch) {
            known = true;
            if state {
                on.push(switch.as_str());
            }
        }
    }
    known.then_some(on)
}

/// Where a quantity crossed `target` between two samples.
///
/// Linear between the two margins, which is the same order of accuracy as the trapezoidal step
/// that produced them -- a higher-order fit here would claim a precision the trace does not
/// carry. Falls back to the sample's own time on the first step, where there is nothing to
/// interpolate from.
fn cross(
    prev_time: Option<f64>,
    prev_margin: Option<f64>,
    t: f64,
    margin: f64,
    target: f64,
) -> f64 {
    let (Some(prev_time), Some(prev_margin)) = (prev_time, prev_margin) else {
        return t;
    };
    let span = margin - prev_margin;
    if span == 0.0 {
        return t;
    }
    let fraction = (target - prev_margin) / span;
    if !(0.0..=1.0).contains(&fraction) {
        return t;
    }
    prev_time + fraction * (t - prev_time)
}

fn enter_record(
    rule: &Rule,
    active: &Active,
    step: usize,
    step_time: f64,
    interactive: bool,
) -> EventRecord {
    EventRecord {
        time: active.started,
        step,
        step_time,
        rule: rule.name.clone(),
        target: rule.target.clone(),
        kind: EventKind::Enter,
        severity: rule.severity,
        action: taken(rule.action, interactive),
        value: active.worst_value,
        limit: active.limit,
        duration: None,
        peak: None,
        peak_time: None,
        truncated: false,
        detail: active.detail.clone(),
        unit: rule.unit.clone(),
    }
}

#[allow(clippy::too_many_arguments)]
fn leave_record(
    rule: &Rule,
    active: &Active,
    ended: f64,
    step: usize,
    step_time: f64,
    truncated: bool,
    interactive: bool,
) -> EventRecord {
    EventRecord {
        time: ended,
        step,
        step_time,
        rule: rule.name.clone(),
        target: rule.target.clone(),
        kind: EventKind::Leave,
        severity: rule.severity,
        action: taken(rule.action, interactive),
        value: active.worst_value,
        limit: active.limit,
        duration: Some((ended - active.started).max(0.0)),
        peak: Some(active.worst_value),
        peak_time: Some(active.worst_time),
        truncated,
        detail: active.detail.clone(),
        unit: rule.unit.clone(),
    }
}

/// What was actually done, which is not always what the rule asked for.
fn taken(action: Action, interactive: bool) -> Action {
    match action {
        Action::Pause if !interactive => Action::Console,
        other => other,
    }
}

/// The worst severity among records, for an exit code.
pub fn worst_severity(records: &[EventRecord]) -> Option<Severity> {
    records.iter().map(|record| record.severity).max()
}
