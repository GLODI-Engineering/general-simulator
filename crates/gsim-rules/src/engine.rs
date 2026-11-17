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

use crate::event::{format_magnitude, Action, EventKind, EventRecord, Severity};
use crate::rule::{PeriodBoundary, PeriodMeasure, Rule, Watch};
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

    /// Whether a commanded-off switch's body diode is carrying current, or `None` when this
    /// sample cannot say -- which includes every commanded-*on* switch, because an on switch
    /// is stamped as a plain resistance and has no body-diode segment to resolve.
    fn body_diode_on(&self, _switch: &str) -> Option<bool> {
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
    /// For [`Watch::SwitchTiming`]: each switch's state on the previous step, when it entered
    /// that state, and how many transitions it has made. Keyed by switch name, so one rule can
    /// watch a whole bridge and report the one leg that misbehaved.
    timing: BTreeMap<String, SwitchHistory>,
    /// The first time any switch of a timing group was readable, so a frequency is measured
    /// over the interval actually observed rather than over `[0, t_end]`.
    timing_since: Option<f64>,
    /// For [`Watch::RateOfChange`]: the quantity's value on the previous step, which is what a
    /// slope is measured from.
    prev_value: Option<f64>,
    /// For [`Watch::CommutationOrder`]: which entry of the sequence should turn on next. Set
    /// by the first turn-on seen rather than starting at zero, so a run that begins midway
    /// through a sequence is not reported for it.
    expected_index: Option<usize>,
    /// For [`Watch::PerPeriod`] and [`Watch::Efficiency`]: the period being accumulated. For an
    /// efficiency this is the numerator's.
    period: Option<Accumulator>,
    /// For [`Watch::Efficiency`] only: the denominator's period, closed on the same boundary as
    /// `period` so the ratio is of two averages over the *same* interval. A single accumulator
    /// would have to be re-run per quantity, and two runs cannot be guaranteed to see the same
    /// boundary once the boundary is an outcome of the circuit rather than a setting.
    period_b: Option<Accumulator>,
    /// Whether a sync boundary has been seen, so the fragment before the first one is not
    /// mistaken for a period.
    period_started: bool,
}

/// Where the period being accumulated stands at this step.
#[derive(Debug, Clone, Copy, PartialEq)]
enum PeriodEdge {
    /// No period is open; this sample opens the first one.
    Opening,
    /// A period is open and continues past this step.
    Continuing,
    /// A period closes at this time, which is usually between two samples rather than on one.
    Closing(f64),
}

/// One period's running integrals, for [`Watch::PerPeriod`].
///
/// Streaming rather than buffered. `gs-waveform-measurements` computes these over a complete
/// slice, and buffering a period per rule is unbounded -- at 10 ns steps a 50 Hz period is two
/// million samples. The per-segment forms below are that crate's own, so a `kind=measure` RMS
/// and a per-period RMS over the same window agree rather than differing in the last digits by
/// a quadrature rule nobody chose.
#[derive(Debug, Clone)]
struct Accumulator {
    /// When this period began.
    started: f64,
    /// The sample at the previous step: the left end of the segment being integrated.
    previous: Option<(f64, f64)>,
    /// $\int x \, \mathrm{d}t$.
    integral: f64,
    /// $\int x^2 \, \mathrm{d}t$.
    integral_squared: f64,
    /// $\int |x| \, \mathrm{d}t$.
    integral_abs: f64,
}

impl Accumulator {
    fn new(started: f64) -> Accumulator {
        Accumulator {
            started,
            previous: None,
            integral: 0.0,
            integral_squared: 0.0,
            integral_abs: 0.0,
        }
    }

    /// Fold the segment from the previous sample to `(t, value)` into the integrals.
    fn extend(&mut self, t: f64, value: f64) {
        if let Some((then, before)) = self.previous {
            let dt = t - then;
            if dt > 0.0 {
                // Trapezoid, which is exact for the linear segment between two samples.
                self.integral += dt * 0.5 * (before + value);
                // The exact integral of the square of that same linear segment -- not
                // `dt/2 * (a^2 + b^2)`, which is the trapezoid of the square and a different
                // number. This is the form `gs_waveform_measurements::rms` uses.
                self.integral_squared +=
                    dt / 3.0 * (before * before + before * value + value * value);
                self.integral_abs += integral_of_abs(dt, before, value);
            }
        }
        self.previous = Some((t, value));
    }

    /// The quantity's value at `ends_at`, from the segment the boundary falls inside.
    ///
    /// A period closes between two samples far more often than on one, and the step that
    /// notices is already past it. Taking this step's value would stretch the last segment
    /// beyond the period; interpolating places the boundary where it belongs, which is what
    /// makes a fixed-period integral independent of the step size that happened to be used.
    fn value_at(&self, ends_at: f64, t: f64, value: f64) -> f64 {
        match self.previous {
            Some((then, before)) if t > then => {
                before + (value - before) * (ends_at - then) / (t - then)
            }
            // Nothing to interpolate from, or two samples at one instant.
            _ => value,
        }
    }

    /// The measure over `[started, ended]`, or `None` for a period of no length.
    fn measure(&self, measure: PeriodMeasure, ended: f64) -> Option<f64> {
        let span = ended - self.started;
        if span <= 0.0 {
            return None;
        }
        Some(match measure {
            PeriodMeasure::Rms => (self.integral_squared / span).sqrt(),
            PeriodMeasure::Mean => self.integral / span,
            PeriodMeasure::AbsMean => self.integral_abs / span,
            PeriodMeasure::Integral => self.integral,
        })
    }
}

/// $\int_0^{\Delta t} |x| \, \mathrm{d}t$ for the linear segment from `a` to `b`.
///
/// Split at the zero crossing when the segment changes sign, because the magnitude of a line
/// that crosses zero is not a line: taking `|a + b| / 2` there would report a cancellation that
/// does not happen, and a symmetric current -- a transformer's, say -- would come out near zero
/// when its mean magnitude is what a thermal estimate needs.
fn integral_of_abs(dt: f64, a: f64, b: f64) -> f64 {
    if (a >= 0.0) == (b >= 0.0) {
        return dt * 0.5 * (a.abs() + b.abs());
    }
    // Where the line crosses zero, as a fraction of the segment.
    let crossing = a.abs() / (a.abs() + b.abs());
    let first = dt * crossing * 0.5 * a.abs();
    let second = dt * (1.0 - crossing) * 0.5 * b.abs();
    first + second
}

/// One switch's transition history, for [`Watch::SwitchTiming`].
#[derive(Debug, Clone, Default)]
struct SwitchHistory {
    /// Its state on the previous step, or `None` before the first.
    on: Option<bool>,
    /// When it entered that state. Not the first step's time for a state it was already in:
    /// an interval that began before the run started has an unknown length and must not be
    /// reported as a short pulse.
    since: Option<f64>,
    /// Completed transitions, either direction. A switching period is two of them.
    transitions: usize,
    /// For [`Watch::Zvs`]: the voltage across this switch on the previous step.
    ///
    /// The previous step is the one that matters, and this field exists because the obvious
    /// implementation is wrong. At the step where the gate *becomes* on, the switch has already
    /// been stamped as an `r_on` resistance, so the voltage across it has already collapsed to
    /// $i\,r_{on}$ -- a first version read it there and reported a hard-switched 400 V leg as
    /// passing. The voltage the switch turned on *into* is the last sample while it was still
    /// off.
    blocking_voltage: Option<f64>,
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
        self.measure_switch_timing(index, t, step, sample, out);
        self.check_commutation_order(index, t, step, sample, out);
        self.measure_period(index, t, step, sample, out);
        self.measure_efficiency(index, t, step, sample, out);
        self.check_zvs(index, t, step, sample, out);

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
        if let Watch::RateOfChange { quantity, .. } = &self.rules[index].watch {
            // The quantity itself, not the margin: the margin is already a slope, and
            // differentiating it would measure an acceleration.
            self.state[index].prev_value = sample.get(quantity);
        }
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
                let value = readable(sample.get(quantity))?;
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
            // A timing rule has no interval of its own: every finding it makes is an instant
            // produced by `measure_switch_timing` or by `finish`. It still has to answer here,
            // because returning `None` is how a sample says it cannot read the switches, and
            // that is what marks a rule NOT EVALUATED. A margin that never violates keeps the
            // interval machinery idle while still recording that the rule was evaluated.
            Watch::SwitchTiming { switches, .. } => {
                let on = conducting(sample, switches)?;
                Some(Observation {
                    value: on.len() as f64,
                    margin: -1.0,
                    limit: 0.0,
                    detail: None,
                })
            }
            Watch::RateOfChange {
                quantity,
                max_slope,
            } => {
                let value = readable(sample.get(quantity))?;
                let state = &self.state[index];
                let slope = match (state.prev_value, state.prev_time) {
                    (Some(previous), Some(then)) if self.last_time > then => {
                        (value - previous) / (self.last_time - then)
                    }
                    // The first step, or two samples at the same instant. There is no slope
                    // yet, and reporting `None` would count the step as unreadable and leave
                    // every rate rule marked NOT EVALUATED for the one step it could never
                    // have answered.
                    _ => 0.0,
                };
                Some(Observation {
                    value: slope,
                    margin: slope.abs() - *max_slope,
                    limit: *max_slope,
                    detail: None,
                })
            }
            Watch::BodyDiode {
                switches,
                i_min,
                currents,
            } => {
                // A switch commanded *on* is a plain resistance with no segment to resolve, so
                // `body_diode_on` says nothing about it -- which is the right answer, not a
                // missing sample. Only a switch that is both off and conducting counts.
                let mut known = false;
                let mut worst = 0.0f64;
                let mut offenders = Vec::new();
                for (position, switch) in switches.iter().enumerate() {
                    let Some(off) = sample.switch_on(switch).map(|on| !on) else {
                        continue;
                    };
                    known = true;
                    if !off || sample.body_diode_on(switch) != Some(true) {
                        continue;
                    }
                    let Some(name) = currents.get(position) else {
                        continue;
                    };
                    let Some(current) = readable(sample.get(name)) else {
                        continue;
                    };
                    if current.abs() > worst {
                        worst = current.abs();
                    }
                    if current.abs() >= *i_min {
                        offenders.push(switch.as_str());
                    }
                }
                if !known {
                    return None;
                }
                Some(Observation {
                    value: worst,
                    margin: worst - *i_min,
                    limit: *i_min,
                    detail: (!offenders.is_empty())
                        .then(|| format!("body diode conducting in {}", offenders.join(", "))),
                })
            }
            // Every finding is an instant produced by `measure_period`; this answers only so
            // that a quantity the sample does not carry is reported as unreadable.
            Watch::PerPeriod {
                quantity, boundary, ..
            } => {
                let value = readable(sample.get(quantity))?;
                if let PeriodBoundary::Sync(switch) = boundary {
                    // A sync switch the netlist does not have makes the rule unanswerable, not
                    // merely quiet: without a boundary no period ever closes.
                    sample.switch_on(switch)?;
                }
                Some(Observation {
                    value,
                    margin: -1.0,
                    limit: 0.0,
                    detail: None,
                })
            }
            // Every finding is an instant produced by `check_zvs`; this answers only so that a
            // switch or a voltage the sample does not carry is reported as unreadable.
            Watch::Zvs {
                switches,
                voltages,
                v_max,
            } => {
                let mut worst = 0.0_f64;
                for (switch, voltage) in switches.iter().zip(voltages.iter()) {
                    sample.switch_on(switch)?;
                    worst = worst.max(readable(sample.get(voltage))?.abs());
                }
                Some(Observation {
                    value: worst,
                    margin: -1.0,
                    limit: *v_max,
                    detail: None,
                })
            }
            // Every finding is an instant produced by `measure_efficiency`; this answers only
            // so that a quantity the sample does not carry is reported as unreadable.
            Watch::Efficiency {
                numerator,
                denominator,
                boundary,
                ..
            } => {
                let value = readable(sample.get(numerator))?;
                readable(sample.get(denominator))?;
                if let PeriodBoundary::Sync(switch) = boundary {
                    // A sync switch the netlist does not have makes the rule unanswerable, not
                    // merely quiet: without a boundary no period ever closes.
                    sample.switch_on(switch)?;
                }
                Some(Observation {
                    value,
                    margin: -1.0,
                    limit: 0.0,
                    detail: None,
                })
            }
            // Every finding is an instant produced by `check_commutation_order`; this answers
            // only so that an unreadable switch is reported as unreadable.
            Watch::CommutationOrder { sequence } => {
                let on = conducting(sample, sequence)?;
                Some(Observation {
                    value: on.len() as f64,
                    margin: -1.0,
                    limit: 0.0,
                    detail: None,
                })
            }
            Watch::RequiredPath {
                switches,
                current,
                i_min,
            } => {
                let on = conducting(sample, switches)?;
                let magnitude = readable(sample.get(current))?.abs();
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

    /// Record each switch's transitions, and report a pulse shorter than the device allows.
    ///
    /// Measured from the resolved states rather than from the modulator's intent, for the same
    /// reason dead time is: what the device did is the only thing that can be too short.
    fn measure_switch_timing(
        &mut self,
        index: usize,
        t: f64,
        step: usize,
        sample: &impl Sample,
        out: &mut Vec<EventRecord>,
    ) {
        let Watch::SwitchTiming {
            switches,
            min_on,
            min_off,
            ..
        } = &self.rules[index].watch
        else {
            return;
        };
        let (min_on, min_off) = (*min_on, *min_off);
        let switches = switches.clone();

        // Each switch separately: a bridge's four switches are one rule, and the report has to
        // name the one that produced the short pulse.
        let mut findings: Vec<(String, bool, f64, f64)> = Vec::new();
        for switch in &switches {
            let Some(now) = sample.switch_on(switch) else {
                continue;
            };
            if self.state[index].timing_since.is_none() {
                self.state[index].timing_since = Some(t);
            }
            let history = self.state[index].timing.entry(switch.clone()).or_default();
            match history.on {
                // First sight of this switch. Its current interval began before the run was
                // watching, so its length is unknown and cannot be judged -- only intervals
                // that both start and end under observation are measured.
                None => {
                    history.on = Some(now);
                    history.since = None;
                }
                Some(was) if was == now => {}
                Some(was) => {
                    history.transitions += 1;
                    if let Some(started) = history.since {
                        let length = t - started;
                        let limit = if was { min_on } else { min_off };
                        if let Some(limit) = limit {
                            if length < limit {
                                findings.push((switch.clone(), was, length, limit));
                            }
                        }
                    }
                    history.on = Some(now);
                    history.since = Some(t);
                }
            }
        }

        for (switch, was_on, length, limit) in findings {
            let what = if was_on { "on" } else { "off" };
            let record = self.instant(
                index,
                t,
                step,
                length,
                limit,
                "s",
                format!("{switch} was {what} for {length:.4e} s, {limit:.4e} s required"),
            );
            self.state[index].instants += 1;
            self.emit(index, record, out);
        }
    }

    /// Report a turn-on that happened out of the stated order.
    ///
    /// The phase is set by the first turn-on observed rather than assumed to be the first
    /// entry of the sequence: a run that begins midway through a commutation is not a run that
    /// got the order wrong, and reporting it would be an artefact of where the window opened.
    fn check_commutation_order(
        &mut self,
        index: usize,
        t: f64,
        step: usize,
        sample: &impl Sample,
        out: &mut Vec<EventRecord>,
    ) {
        let Watch::CommutationOrder { sequence } = &self.rules[index].watch else {
            return;
        };
        let sequence = sequence.clone();
        if sequence.is_empty() {
            return;
        }

        // Turn-ons only. A sequence states the order devices take over the current in, and a
        // turn-off is the consequence of the next one taking it.
        let mut turned_on: Vec<(usize, String)> = Vec::new();
        for (position, switch) in sequence.iter().enumerate() {
            let Some(now) = sample.switch_on(switch) else {
                continue;
            };
            let history = self.state[index].timing.entry(switch.clone()).or_default();
            let was = history.on;
            history.on = Some(now);
            if was == Some(false) && now {
                turned_on.push((position, switch.clone()));
            }
        }

        for (position, switch) in turned_on {
            match self.state[index].expected_index {
                None => {}
                Some(expected) if expected == position => {}
                Some(expected) => {
                    let wanted = &sequence[expected];
                    let record = self.instant(
                        index,
                        t,
                        step,
                        position as f64,
                        expected as f64,
                        "",
                        format!(
                            "{switch} turned on out of order -- {wanted} was expected next in \
                             {}",
                            sequence.join(" -> ")
                        ),
                    );
                    self.state[index].instants += 1;
                    self.emit(index, record, out);
                }
            }
            // Resynchronised to what actually happened, not to what was wanted: otherwise one
            // departure reports again on every later transition, and a log with a thousand
            // lines in it hides the one that mattered.
            self.state[index].expected_index = Some((position + 1) % sequence.len());
        }
    }

    /// Accumulate the current period and report one that falls outside its bounds.
    ///
    /// A fixed period closes by *splitting* the step that crosses the boundary: the sample at
    /// the boundary is interpolated between the two steps either side of it, the part before
    /// belongs to the period that is closing, and the part after opens the next. Closing at the
    /// step that noticed instead would measure one period plus however much of a step the
    /// stepping happened to leave, and for volt-second balance that is not a small error -- the
    /// quantity being bounded is near zero, so a stray step's worth of volt-seconds is not
    /// small compared with it.
    /// Decides the period boundary for one rule at `t`, recording a sync switch's transition.
    ///
    /// Shared by [`Self::measure_period`] and [`Self::measure_efficiency`] because the decision
    /// has two cases that are easy to get wrong and were each found by a failing test: the
    /// first turn-on of a sync boundary opens the first period rather than closing one, and a
    /// fixed boundary closes between samples rather than on one. Duplicating that in a second
    /// measurement would have duplicated the mistakes.
    ///
    /// `open_started` is when the currently open period began, or `None` if none is open; the
    /// caller owns the accumulators, so it answers that question rather than this.
    ///
    /// `None` means the boundary itself is unreadable or meaningless -- a sync switch the
    /// netlist does not have, or a period of zero length -- and nothing can be measured.
    fn period_edge(
        &mut self,
        index: usize,
        t: f64,
        boundary: &PeriodBoundary,
        sample: &impl Sample,
        open_started: Option<f64>,
    ) -> Option<PeriodEdge> {
        // A sync boundary has to see the switch transition whether or not a period is open, so
        // the transition is recorded first and its answer used below.
        let synced = match boundary {
            PeriodBoundary::Sync(switch) => {
                let now = sample.switch_on(switch)?;
                let history = self.state[index].timing.entry(switch.clone()).or_default();
                let was = history.on;
                history.on = Some(now);
                was == Some(false) && now
            }
            PeriodBoundary::Fixed(period) if *period <= 0.0 => return None,
            PeriodBoundary::Fixed(_) => false,
        };

        // With a sync boundary the first turn-on does not close a period, it establishes where
        // the first one starts: the interval from the run's beginning to that turn-on is a
        // fragment of a period that began before anything was watching, and measuring it would
        // report a volt-second balance over whatever fraction of a cycle the run happened to
        // open in. Same discipline as the first switch interval in `measure_switch_timing`.
        if synced && !self.state[index].period_started {
            self.state[index].period_started = true;
            return Some(PeriodEdge::Opening);
        }

        // The first sample opens the first period. Nothing before it is judged: a period that
        // began before the run was watching has no measurable start.
        let Some(started) = open_started else {
            return Some(PeriodEdge::Opening);
        };

        Some(match boundary {
            PeriodBoundary::Sync(_) if synced => PeriodEdge::Closing(t),
            PeriodBoundary::Sync(_) => PeriodEdge::Continuing,
            PeriodBoundary::Fixed(period) => {
                let ends_at = started + *period;
                if t >= ends_at {
                    PeriodEdge::Closing(ends_at)
                } else {
                    PeriodEdge::Continuing
                }
            }
        })
    }

    /// The voltage standing across each switch at the instant it is commanded on:
    /// [`Watch::Zvs`].
    ///
    /// Reuses the same `timing` history as [`Self::measure_switch_timing`], so a rule that
    /// watches a whole bridge reports the one leg that hard-switched rather than an average over
    /// the group. The first sample of a switch establishes its state without reporting: a run
    /// that opens with a switch already on has no transition to measure, and reporting the
    /// opening state would report the step size rather than the circuit.
    fn check_zvs(
        &mut self,
        index: usize,
        t: f64,
        step: usize,
        sample: &impl Sample,
        out: &mut Vec<EventRecord>,
    ) {
        let Watch::Zvs {
            switches,
            voltages,
            v_max,
        } = &self.rules[index].watch
        else {
            return;
        };
        let v_max = *v_max;
        let pairs: Vec<(String, String)> = switches
            .iter()
            .cloned()
            .zip(voltages.iter().cloned())
            .collect();

        for (switch, voltage) in pairs {
            let Some(now) = sample.switch_on(&switch) else {
                continue;
            };
            let reading = readable(sample.get(&voltage));
            let history = self.state[index].timing.entry(switch.clone()).or_default();
            let was = history.on;
            history.on = Some(now);
            // The voltage from *before* this step, kept while the switch is off. See
            // `SwitchHistory::blocking_voltage` for why reading it at the transition is wrong.
            let before = history.blocking_voltage;
            if !now {
                history.blocking_voltage = reading;
            }
            // Only an off-to-on transition is a turn-on. `was == None` is the first sample.
            if was != Some(false) || !now {
                continue;
            }
            let Some(across) = before else {
                continue;
            };
            // The magnitude: a switch may stand off either polarity depending on how its two
            // terminals were declared, and the design question is about how far from zero the
            // commutation got, not which way round the node pair was written.
            let magnitude = across.abs();
            if magnitude <= v_max {
                continue;
            }
            let unit = self.rules[index].unit.clone();
            let record = self.instant(
                index,
                t,
                step,
                magnitude,
                v_max,
                &unit,
                format!("{switch} turned on with {voltage} still across it"),
            );
            self.state[index].instants += 1;
            self.emit(index, record, out);
        }
    }

    /// One quantity, averaged or integrated over each period: [`Watch::PerPeriod`].
    fn measure_period(
        &mut self,
        index: usize,
        t: f64,
        step: usize,
        sample: &impl Sample,
        out: &mut Vec<EventRecord>,
    ) {
        let Watch::PerPeriod {
            quantity,
            measure,
            boundary,
            min,
            max,
        } = &self.rules[index].watch
        else {
            return;
        };
        let (measure, min, max) = (*measure, *min, *max);
        let boundary = boundary.clone();
        let quantity = quantity.clone();
        let Some(value) = readable(sample.get(&quantity)) else {
            return;
        };

        let started = self.state[index].period.as_ref().map(|open| open.started);
        let Some(edge) = self.period_edge(index, t, &boundary, sample, started) else {
            return;
        };

        let ends_at = match edge {
            PeriodEdge::Opening => {
                let mut first = Accumulator::new(t);
                first.extend(t, value);
                self.state[index].period = Some(first);
                return;
            }
            PeriodEdge::Continuing => {
                // `period` is `Some` here: `Opening` is the only edge returned without one.
                if let Some(open) = self.state[index].period.as_mut() {
                    open.extend(t, value);
                }
                return;
            }
            PeriodEdge::Closing(ends_at) => ends_at,
        };

        let Some(open) = self.state[index].period.as_mut() else {
            return;
        };
        let at_boundary = open.value_at(ends_at, t, value);

        // Close the period at the boundary, not at this step.
        open.extend(ends_at, at_boundary);
        let result = open.measure(measure, ends_at);
        let started = open.started;

        // Open the next period at the boundary, carrying the boundary sample as its left end so
        // the segment straddling two periods is split between them rather than dropped.
        let mut next = Accumulator::new(ends_at);
        next.extend(ends_at, at_boundary);
        if t > ends_at {
            next.extend(t, value);
        }
        self.state[index].period = Some(next);

        let Some(result) = result else {
            return;
        };
        let outside = match (min, max) {
            (Some(low), _) if result < low => Some(low),
            (_, Some(high)) if result > high => Some(high),
            _ => None,
        };
        if let Some(limit) = outside {
            let unit = self.rules[index].unit.clone();
            let record = self.instant(
                index,
                ends_at,
                step,
                result,
                limit,
                &unit,
                format!(
                    "{} over the period {started:e} s .. {ends_at:e} s",
                    describe_measure(measure, &quantity)
                ),
            );
            self.state[index].instants += 1;
            self.emit(index, record, out);
        }
    }

    /// The ratio of two quantities' per-period means: [`Watch::Efficiency`].
    ///
    /// Both accumulate over the same boundary and both close at the same instant, which is the
    /// reason this is one measurement rather than two `PerPeriod` rules and a division: with a
    /// `Sync` boundary the period is an outcome of the circuit, so two separately-run
    /// measurements have no guarantee of agreeing on where a period ended.
    fn measure_efficiency(
        &mut self,
        index: usize,
        t: f64,
        step: usize,
        sample: &impl Sample,
        out: &mut Vec<EventRecord>,
    ) {
        let Watch::Efficiency {
            numerator,
            denominator,
            boundary,
            min,
            max,
        } = &self.rules[index].watch
        else {
            return;
        };
        let (min, max) = (*min, *max);
        let boundary = boundary.clone();
        let (numerator, denominator) = (numerator.clone(), denominator.clone());
        // Either quantity unreadable makes the ratio unreadable -- not "not violating".
        let (Some(top), Some(bottom)) = (
            readable(sample.get(&numerator)),
            readable(sample.get(&denominator)),
        ) else {
            return;
        };

        let started = self.state[index].period.as_ref().map(|open| open.started);
        let Some(edge) = self.period_edge(index, t, &boundary, sample, started) else {
            return;
        };

        let ends_at = match edge {
            PeriodEdge::Opening => {
                let mut first = Accumulator::new(t);
                first.extend(t, top);
                let mut second = Accumulator::new(t);
                second.extend(t, bottom);
                self.state[index].period = Some(first);
                self.state[index].period_b = Some(second);
                return;
            }
            PeriodEdge::Continuing => {
                if let Some(open) = self.state[index].period.as_mut() {
                    open.extend(t, top);
                }
                if let Some(open) = self.state[index].period_b.as_mut() {
                    open.extend(t, bottom);
                }
                return;
            }
            PeriodEdge::Closing(ends_at) => ends_at,
        };

        let (Some(mut open_top), Some(mut open_bottom)) = (
            self.state[index].period.take(),
            self.state[index].period_b.take(),
        ) else {
            return;
        };

        let top_at = open_top.value_at(ends_at, t, top);
        let bottom_at = open_bottom.value_at(ends_at, t, bottom);
        open_top.extend(ends_at, top_at);
        open_bottom.extend(ends_at, bottom_at);
        let mean_top = open_top.measure(PeriodMeasure::Mean, ends_at);
        let mean_bottom = open_bottom.measure(PeriodMeasure::Mean, ends_at);
        let started = open_top.started;

        let mut next_top = Accumulator::new(ends_at);
        next_top.extend(ends_at, top_at);
        let mut next_bottom = Accumulator::new(ends_at);
        next_bottom.extend(ends_at, bottom_at);
        if t > ends_at {
            next_top.extend(t, top);
            next_bottom.extend(t, bottom);
        }
        self.state[index].period = Some(next_top);
        self.state[index].period_b = Some(next_bottom);

        let (Some(mean_top), Some(mean_bottom)) = (mean_top, mean_bottom) else {
            return;
        };
        // A period whose denominator averages to zero is skipped, not reported. A ratio against
        // nothing says nothing about the circuit, and a converter whose input power has not
        // arrived yet would otherwise be reported on its very first period -- for being off.
        if mean_bottom == 0.0 {
            return;
        }
        let ratio = mean_top / mean_bottom;
        if !ratio.is_finite() {
            return;
        }

        let outside = match (min, max) {
            (Some(low), _) if ratio < low => Some(low),
            (_, Some(high)) if ratio > high => Some(high),
            _ => None,
        };
        if let Some(limit) = outside {
            let unit = self.rules[index].unit.clone();
            let record = self.instant(
                index,
                ends_at,
                step,
                ratio,
                limit,
                &unit,
                format!(
                    "mean {numerator} / mean {denominator} = {} / {} over the period \
                     {started:e} s .. {ends_at:e} s",
                    format_magnitude(mean_top),
                    format_magnitude(mean_bottom)
                ),
            );
            self.state[index].instants += 1;
            self.emit(index, record, out);
        }
    }

    /// An [`EventKind::Instant`] finding: something measured at a transition, with no interval.
    #[allow(clippy::too_many_arguments)]
    fn instant(
        &self,
        index: usize,
        t: f64,
        step: usize,
        value: f64,
        limit: f64,
        unit: &str,
        detail: String,
    ) -> EventRecord {
        let rule = &self.rules[index];
        EventRecord {
            time: t,
            step,
            step_time: t,
            rule: rule.name.clone(),
            target: rule.target.clone(),
            kind: EventKind::Instant,
            severity: rule.severity,
            action: taken(rule.action, self.interactive),
            value,
            limit,
            duration: None,
            peak: None,
            peak_time: None,
            truncated: false,
            detail: Some(detail),
            unit: unit.to_string(),
        }
    }

    /// The end-of-run half of [`Watch::SwitchTiming`]: a frequency is only known once the run
    /// is over, and a switch that never commutated can only be reported then.
    fn finish_switch_timing(&mut self, index: usize, step: usize, out: &mut Vec<EventRecord>) {
        let Watch::SwitchTiming {
            switches,
            f_min,
            f_max,
            require_commutation,
            ..
        } = &self.rules[index].watch
        else {
            return;
        };
        let (f_min, f_max, require_commutation) = (*f_min, *f_max, *require_commutation);
        let switches = switches.clone();
        let t = self.last_time;
        let span = match self.state[index].timing_since {
            Some(since) if t > since => t - since,
            // Nothing was observed, or one step was: a frequency over a zero-length window is
            // not a measurement, and reporting one would be inventing it.
            _ => 0.0,
        };

        let mut findings: Vec<(f64, f64, String)> = Vec::new();
        for switch in &switches {
            let Some(history) = self.state[index].timing.get(switch) else {
                continue;
            };
            let transitions = history.transitions;
            if require_commutation && transitions == 0 {
                let held = match history.on {
                    Some(true) => "conducting",
                    Some(false) => "open",
                    None => "unreadable",
                };
                findings.push((
                    0.0,
                    1.0,
                    format!("{switch} never commutated -- it stayed {held} for the whole run"),
                ));
                continue;
            }
            if span == 0.0 || transitions == 0 {
                continue;
            }
            // Two transitions per period. A half-cycle at the end of the run contributes its
            // half, which is why this is not an integer count of completed periods.
            let frequency = transitions as f64 / 2.0 / span;
            if let Some(limit) = f_min {
                if frequency < limit {
                    findings.push((
                        frequency,
                        limit,
                        format!("{switch} switched at {frequency:.4e} Hz, below {limit:.4e} Hz"),
                    ));
                }
            }
            if let Some(limit) = f_max {
                if frequency > limit {
                    findings.push((
                        frequency,
                        limit,
                        format!("{switch} switched at {frequency:.4e} Hz, above {limit:.4e} Hz"),
                    ));
                }
            }
        }

        for (value, limit, detail) in findings {
            let unit = if detail.contains("never commutated") {
                " transition(s)"
            } else {
                " Hz"
            };
            let record = self.instant(index, t, step, value, limit, unit, detail);
            self.state[index].instants += 1;
            self.emit(index, record, out);
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
            // `target.name`, the same label every report prints, rather than the rule's own
            // name: a card's name is unique in a deck, but a rating's is the field it was
            // written as, so "stopped by rule 'i_max'" would not say which component.
            self.stop_requested = Some(format!("{}.{}", rule.target, rule.name));
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
            self.finish_switch_timing(index, step, &mut out);
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

/// How to name a per-period measure in a report.
fn describe_measure(measure: PeriodMeasure, quantity: &str) -> String {
    match measure {
        PeriodMeasure::Rms => format!("RMS of {quantity}"),
        PeriodMeasure::Mean => format!("mean of {quantity}"),
        PeriodMeasure::AbsMean => format!("mean |{quantity}|"),
        PeriodMeasure::Integral => format!("integral of {quantity}"),
    }
}

/// A sample's value, or `None` when there is nothing usable there.
///
/// A non-finite value is an unreadable sample, not a value that happens not to violate. Every
/// comparison against a `NaN` is false -- `NaN > limit`, `NaN < limit`, both -- so without this
/// a rule whose quantity has gone to `NaN` finds no violation and the summary reports `passed`:
/// the question was asked, the answer was unobtainable, and the table says the circuit is fine.
/// Counted as missing instead, which is what `NOT EVALUATED` means and what `passed()` refuses.
///
/// The CLI also abandons a run whose state goes non-finite (see its `finite` module), which is
/// the better answer. This is the engine's own guard, for every other caller and for the case
/// where only a block output nobody stops on has broken.
fn readable(value: Option<f64>) -> Option<f64> {
    value.filter(|v| v.is_finite())
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
