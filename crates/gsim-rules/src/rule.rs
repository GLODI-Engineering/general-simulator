//! What a rule is: the quantity it watches, the condition that makes it fire, the qualifiers
//! that decide when a true condition actually counts, and what to do about it.

use serde::{Deserialize, Serialize};

use crate::event::{Action, Severity};

/// When a watched quantity counts as violating.
///
/// Every variant answers one question -- "how far past the line is this value?" -- through
/// [`Condition::margin`], and the rest of the engine is written against that number alone. It
/// is what lets one piece of code do edge detection, hysteresis and sub-step interpolation for
/// all four shapes instead of four times over, and what makes adding a fifth cheap.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Condition {
    /// Violating above `limit`.
    Above { limit: f64 },
    /// Violating below `limit`.
    Below { limit: f64 },
    /// Violating outside the band, on either side.
    Outside { low: f64, high: f64 },
    /// Violating while inside the band -- a signal that is meant to stay away from a region,
    /// such as a dead-time window a transition must not land in.
    Inside { low: f64, high: f64 },
}

impl Condition {
    /// How far into violation `value` is: positive while violating, negative while not, and
    /// zero exactly on the line.
    ///
    /// The number is in the quantity's own unit, which is what makes a hysteresis band
    /// expressible in amps rather than as a fraction nobody can check against a datasheet.
    pub fn margin(self, value: f64) -> f64 {
        match self {
            Condition::Above { limit } => value - limit,
            Condition::Below { limit } => limit - value,
            // Outside: violating when past either edge, and the margin is the larger excursion
            // so that a value far below `low` does not report a small margin.
            Condition::Outside { low, high } => (low - value).max(value - high),
            // Inside: violating while between the edges; the margin is the distance to the
            // nearer edge, which is how far the value would have to move to stop violating.
            Condition::Inside { low, high } => (value - low).min(high - value),
        }
    }

    /// The limit a report should quote for this value -- the edge that was actually crossed.
    pub fn reference(self, value: f64) -> f64 {
        match self {
            Condition::Above { limit } | Condition::Below { limit } => limit,
            Condition::Outside { low, high } => {
                if (low - value) >= (value - high) {
                    low
                } else {
                    high
                }
            }
            Condition::Inside { low, high } => {
                if (value - low) <= (high - value) {
                    low
                } else {
                    high
                }
            }
        }
    }

    /// Whether a larger value is worse, which decides what "peak" means in a leave event.
    ///
    /// For a band there is no single direction, so the worst value is tracked by margin
    /// instead; see [`crate::engine`].
    pub fn larger_is_worse(self) -> bool {
        matches!(self, Condition::Above { .. })
    }
}

/// The qualifiers that stop a technically-true condition from being noise.
///
/// Every one of these exists because a real run produced a log nobody could read without it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct Qualifiers {
    /// The condition must hold this long before an enter is emitted. A 20 ns numerical spike
    /// from a switch model then raises nothing unless it was asked to.
    ///
    /// The enter's reported time is still the crossing, not the moment the hold expired: the
    /// question a reader asks is when it started, and the hold is a filter on whether to report
    /// at all, not a redefinition of when it happened.
    pub min_duration: Option<f64>,
    /// Leave only once the value has come back past the limit by this much. Stops a quantity
    /// sitting exactly on its limit from producing an enter and a leave every step.
    pub hysteresis: Option<f64>,
    /// Ignore the rule before this time -- skipping start-up, most often.
    pub after: Option<f64>,
    /// Ignore the rule after this time.
    pub before: Option<f64>,
    /// Stop writing per-event output after this many entries; the summary keeps counting them
    /// all. A rule that fires ten thousand times should not be able to bury the one that fired
    /// once.
    pub max_events: Option<usize>,
    /// Turn the action into a stop only once the rule has entered this many times. A first bad
    /// commutation can be tolerable where a hundred are not.
    pub stop_after: Option<usize>,
}

/// What a rule looks at.
///
/// A rule that spans several components reduces to the same machinery as one watching a single
/// quantity, because every variant answers the same question -- how far into violation are we,
/// in a unit that can be interpolated across a step. Two switches of a leg conducting together
/// is "how many are on, against how many may be": a count, with the same edges, hold time and
/// duration as an over-current. That is why there is one engine rather than one per rule type,
/// and why the next rule type costs a variant rather than a state machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "watch", rename_all = "snake_case")]
pub enum Watch {
    /// One named scalar read from the sample, judged by the rule's [`Condition`].
    Quantity { quantity: String },
    /// A set of switches that must never conduct at the same time.
    ///
    /// The classic case is the two switches of one leg: both on is a shoot-through, a short
    /// across the DC link that an ideal-switch model renders as an enormous current rather than
    /// as the destroyed hardware it would be. `max_on` is how many may conduct together --
    /// 1 for a half-bridge leg.
    ForbiddenState {
        switches: Vec<String>,
        max_on: usize,
        /// Fire when the gap between one switch opening and the next closing is shorter than
        /// this. Measured from the real transitions, not from what the modulator intended.
        min_deadtime: Option<f64>,
    },
    /// A set of switches of which at least one must conduct while a current is flowing.
    ///
    /// The opposite mistake, and the one that is easy to miss because the waveform looks
    /// dramatic rather than wrong: opening every switch in an inductor's path gives the current
    /// nowhere to go, and the solver answers with a voltage spike instead of an error. In the
    /// IMDAB3R the secondary bridge must never open both switches of a phase while the
    /// transformer current flows.
    RequiredPath {
        switches: Vec<String>,
        /// The current that must have somewhere to go.
        current: String,
        /// Below this magnitude there is nothing to strand, so the rule stays quiet.
        i_min: f64,
    },
    /// When a group of switches changes state, rather than what state it is in.
    ///
    /// Three questions with one answer, which is why they are one watch: all three are read
    /// off the same record of resolved transitions, and anyone who asks one usually wants the
    /// others.
    ///
    /// A waveform hides every one of them. A 50 ns pulse and a 500 ns pulse are the same
    /// vertical line at any scale a person plots; a gate that never moves looks like a gate
    /// that was never meant to; and a switching frequency that drifted under load looks
    /// exactly like the frequency that was asked for.
    SwitchTiming {
        switches: Vec<String>,
        /// Report a conducting interval shorter than this. A real device has a minimum
        /// on-time -- driver propagation, bootstrap charging, current-sense blanking -- and a
        /// modulator asking for less does not get a short pulse, it gets an unpredictable one.
        min_on: Option<f64>,
        /// Report a non-conducting interval shorter than this, for the same reasons.
        min_off: Option<f64>,
        /// Report a measured switching frequency below this. With a closed loop the frequency
        /// is an outcome rather than a setting, and the magnetics, the losses and the filter
        /// were all designed for a number nobody checked afterwards.
        f_min: Option<f64>,
        /// Report a measured switching frequency above this.
        f_max: Option<f64>,
        /// Report a switch that held one state for the whole run.
        ///
        /// The most common cause of a converter simulation that produces a plausible wrong
        /// answer: the modulator never reaches the gate, a `ctrl=` names the wrong block, the
        /// signal never crosses the gate binding's threshold. What then runs is a different
        /// circuit -- usually one with a permanently open switch, which still charges and
        /// still settles.
        require_commutation: bool,
    },
    /// How fast a quantity is allowed to change.
    ///
    /// The general form of the cut-current check. Open every switch in an inductor's path and
    /// the current has nowhere to go; the solver answers with whatever voltage the stamp
    /// implies rather than with an error, and the result is the enormous spike that everyone
    /// has seen and that is routinely blamed on the solver rather than on the netlist.
    ///
    /// `RequiredPath` catches the case where the switches that should have been conducting can
    /// be named. This catches it from the other side, off the solution itself, and catches
    /// every other impossible slew with it -- a current step through an inductance implies
    /// $v = L \, \mathrm{d}i/\mathrm{d}t$, and a bound on the slope is a bound on that
    /// voltage.
    RateOfChange {
        quantity: String,
        /// Report when $|\mathrm{d}x/\mathrm{d}t|$ exceeds this, in the quantity's own units
        /// per second.
        max_slope: f64,
    },
    /// A switch that is commanded off and conducting anyway, through its body diode.
    ///
    /// Not a fault by itself -- it is exactly how a synchronous bridge freewheels, and how
    /// every unsynchronised dead time is survived. It becomes a finding when the current is
    /// large, because then it is either a sync FET whose channel was never turned on (so the
    /// losses are the diode's, several times what the design assumed) or a path nobody
    /// intended.
    ///
    /// Only meaningful for a commanded-off switch, and that is not a limitation of the rule
    /// but of what the question means: a switch commanded *on* is stamped as a plain
    /// resistance conducting in both directions, so it cannot fail to conduct.
    BodyDiode {
        switches: Vec<String>,
        /// Below this magnitude the conduction is ordinary dead-time freewheeling, so the rule
        /// stays quiet.
        i_min: f64,
        /// The current through each switch, in `switches` order. Named rather than derived
        /// because a switch's own current is not an MNA unknown.
        currents: Vec<String>,
    },
    /// A quantity averaged over one switching period, rather than at an instant.
    ///
    /// The quantities that say whether a converter is working are averages over a period, and
    /// no instantaneous check can express them. The motivating case is a real one: a
    /// transformer's magnetising current drifting from 40 A to 245 A over a run, while every
    /// instantaneous rule passes throughout. Nothing is over a limit at any moment. What is
    /// wrong is that the volt-seconds applied to the winding do not sum to zero over a period,
    /// so each cycle leaves a little more flux than the last.
    PerPeriod {
        quantity: String,
        measure: PeriodMeasure,
        boundary: PeriodBoundary,
        /// Report a period whose measure falls below this.
        min: Option<f64>,
        /// Report a period whose measure rises above this.
        max: Option<f64>,
    },
    /// The ratio of two quantities, each averaged over the same period.
    ///
    /// Built for efficiency, which is what the question "is this a converter at all" reduces
    /// to. A converter that produces more power than it consumes is not a converter, and
    /// nothing in a waveform says so: every voltage and current can sit inside its own limit
    /// while the circuit as a whole invents energy.
    ///
    /// The quantity is deliberately a *ratio* rather than a bounded difference between the two
    /// powers. A difference needs a tolerance, and a tolerance needs a reference -- a
    /// percentage of the input, of the larger side, of the rated power -- and each choice makes
    /// a different claim. At 5% load, 2% of the input power is a different statement from 2% of
    /// the rating, and neither is the one anyone means. A ratio is dimensionless and
    /// self-normalising, so $\eta \le 1$ is the same correct claim at full load and at no load.
    ///
    /// Both quantities are averaged because instantaneous power changes sign inside a
    /// switching cycle: the useful statement is about the cycle, not the instant.
    ///
    /// It also catches the lossless-element mistakes, which no efficiency *floor* would. A
    /// transformer stamped with the wrong turns ratio dissipates nothing, so it does not look
    /// lossy -- it looks like a gain. A band of $0.99 \le \eta \le 1.01$ across such an element
    /// reports it; so does a dependent source with the wrong gain, and a port wired to the
    /// wrong node.
    Efficiency {
        /// The quantity on top -- the output power, for an efficiency.
        numerator: String,
        /// The quantity underneath -- the input power. A period in which this averages to zero
        /// is skipped rather than reported: a ratio against nothing is not a finding about the
        /// circuit, and a converter that has not started yet would otherwise fail on its first
        /// period.
        denominator: String,
        boundary: PeriodBoundary,
        /// Report a period whose ratio falls below this -- an efficiency floor.
        min: Option<f64>,
        /// Report a period whose ratio rises above this. `max=1` is the one that says "this is
        /// a converter": above it, energy is being created.
        max: Option<f64>,
    },
    /// The order a group of switches must turn on in.
    ///
    /// Multi-step commutation -- a current-source inverter, a matrix converter, any four-step
    /// sequence -- is correct only if the transitions happen in the stated order. Out of order
    /// it still simulates, and the answer is wrong in a way nothing in the waveform reveals.
    CommutationOrder {
        /// The cyclic order of turn-ons. After the last, the next expected is the first again.
        sequence: Vec<String>,
    },
}

/// What to compute over one period.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeriodMeasure {
    /// Root mean square -- what a component is rated by, as opposed to its peak.
    Rms,
    /// The plain average, for a DC-link current or an output voltage.
    Mean,
    /// The average of the magnitude, which is what a thermal estimate wants.
    AbsMean,
    /// The integral over the period: $\int_t^{t+T} x \, \mathrm{d}t$.
    ///
    /// For a winding voltage this is the volt-second balance, and it should be zero. A bound on
    /// how far from zero it may be is the check that catches a flux walk-away.
    Integral,
}

/// Where one period ends and the next begins.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeriodBoundary {
    /// A stated period, for an open-loop converter at a fixed frequency.
    Fixed(f64),
    /// Each interval between successive turn-ons of a named switch.
    ///
    /// With a closed loop the period is an outcome rather than a setting, so the only honest
    /// boundary is the one the circuit actually produced.
    Sync(String),
}

impl Watch {
    /// Every component this rule reads, for reporting and for checking it against the netlist.
    pub fn components(&self) -> Vec<&str> {
        match self {
            Watch::Quantity { quantity } => vec![quantity.as_str()],
            Watch::ForbiddenState { switches, .. } => switches.iter().map(String::as_str).collect(),
            Watch::SwitchTiming { switches, .. } => switches.iter().map(String::as_str).collect(),
            Watch::RateOfChange { quantity, .. } => vec![quantity.as_str()],
            Watch::BodyDiode { switches, .. } => switches.iter().map(String::as_str).collect(),
            Watch::CommutationOrder { sequence } => sequence.iter().map(String::as_str).collect(),
            Watch::PerPeriod {
                quantity, boundary, ..
            } => match boundary {
                PeriodBoundary::Fixed(_) => vec![quantity.as_str()],
                PeriodBoundary::Sync(switch) => vec![quantity.as_str(), switch.as_str()],
            },
            Watch::Efficiency {
                numerator,
                denominator,
                boundary,
                ..
            } => {
                let mut names = vec![numerator.as_str(), denominator.as_str()];
                if let PeriodBoundary::Sync(switch) = boundary {
                    names.push(switch.as_str());
                }
                names
            }
            Watch::RequiredPath {
                switches, current, ..
            } => switches
                .iter()
                .map(String::as_str)
                .chain(std::iter::once(current.as_str()))
                .collect(),
        }
    }
}

/// One rule: what to watch, when it counts, and what to do.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    /// The rule's own name, as the netlist card gave it.
    pub name: String,
    /// What is being watched, for the report: a component, switch or signal name.
    pub target: String,
    /// What the rule reads each step through [`crate::Sample`].
    pub watch: Watch,
    /// The unit, used only when printing. This crate never interprets it.
    pub unit: String,
    pub condition: Condition,
    pub qualifiers: Qualifiers,
    pub severity: Severity,
    pub action: Action,
}

impl Rule {
    /// A rule with no qualifiers -- the common case, and the one tests should not have to spell.
    pub fn new(
        name: impl Into<String>,
        target: impl Into<String>,
        quantity: impl Into<String>,
        condition: Condition,
        severity: Severity,
        action: Action,
    ) -> Rule {
        Rule {
            name: name.into(),
            target: target.into(),
            watch: Watch::Quantity {
                quantity: quantity.into(),
            },
            unit: String::new(),
            condition,
            qualifiers: Qualifiers::default(),
            severity,
            action,
        }
    }

    /// Whether `t` is inside the rule's window.
    pub fn in_window(&self, t: f64) -> bool {
        if let Some(after) = self.qualifiers.after {
            if t < after {
                return false;
            }
        }
        if let Some(before) = self.qualifiers.before {
            if t > before {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn margin_is_positive_exactly_while_violating() {
        let above = Condition::Above { limit: 50.0 };
        assert!(above.margin(52.0) > 0.0);
        assert_eq!(above.margin(50.0), 0.0);
        assert!(above.margin(48.0) < 0.0);

        let below = Condition::Below { limit: -5.0 };
        assert!(below.margin(-6.0) > 0.0);
        assert!(below.margin(-4.0) < 0.0);

        let outside = Condition::Outside {
            low: 390.0,
            high: 410.0,
        };
        assert!(outside.margin(380.0) > 0.0);
        assert!(outside.margin(400.0) < 0.0);
        assert!(outside.margin(420.0) > 0.0);

        let inside = Condition::Inside {
            low: 0.0,
            high: 1.0,
        };
        assert!(inside.margin(0.5) > 0.0);
        assert!(inside.margin(-1.0) < 0.0);
        assert!(inside.margin(2.0) < 0.0);
    }

    #[test]
    fn an_outside_band_reports_the_edge_that_was_actually_crossed() {
        let band = Condition::Outside {
            low: 390.0,
            high: 410.0,
        };
        assert_eq!(band.reference(380.0), 390.0);
        assert_eq!(band.reference(420.0), 410.0);
    }

    #[test]
    fn an_outside_bands_margin_is_the_larger_excursion_not_the_nearer_edge() {
        // A bus at 100 V against a 390-410 V band is 290 V out, not 310 V the other way: the
        // margin has to name how far past the crossed edge it is, since a leave is decided by
        // the margin coming back through zero.
        let band = Condition::Outside {
            low: 390.0,
            high: 410.0,
        };
        assert_eq!(band.margin(100.0), 290.0);
    }

    #[test]
    fn a_window_excludes_the_times_outside_it() {
        let mut rule = Rule::new(
            "r",
            "L1",
            "i(L1)",
            Condition::Above { limit: 1.0 },
            Severity::Warning,
            Action::Log,
        );
        rule.qualifiers.after = Some(1e-3);
        rule.qualifiers.before = Some(5e-3);
        assert!(!rule.in_window(0.5e-3));
        assert!(rule.in_window(1e-3), "the boundary is inside the window");
        assert!(rule.in_window(3e-3));
        assert!(rule.in_window(5e-3));
        assert!(!rule.in_window(6e-3));
    }
}
