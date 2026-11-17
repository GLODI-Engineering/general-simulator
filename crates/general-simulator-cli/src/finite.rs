//! The check that has to come before the others: is the solution still made of numbers?
//!
//! Every rule compares a value against a limit, and every such comparison against a `NaN` is
//! `false`. So a run whose state has gone non-finite reports nothing, summarises as `passed`,
//! and exits 0 -- while its waveform file carries a column of `NaN` that a plotting tool draws
//! as a gap and a reader takes for a rendering artefact.
//!
//! That makes this the one check that is on by default. Every other rule is written by someone
//! who suspected something; nobody suspects a `NaN`, and a run that produced one has no result
//! to report, so there is nothing an opt-in would protect.

use std::collections::BTreeMap;

use dae_runtime::{OperatingPoint, SignalValue};
use gsim_rules::{Action, EventKind, EventRecord, Severity};

/// The first non-finite unknown in `point`, if any: its name and its value.
///
/// Which unknown it is matters. A `NaN` that appears first in a branch current and one that
/// appears first in a node voltage point at different causes, and the name is the only thing
/// in the report that sends a reader anywhere.
pub fn first_non_finite(point: &OperatingPoint) -> Option<(&str, f64)> {
    point
        .unknowns
        .iter()
        .zip(point.x.iter())
        .find(|(_, value)| !value.is_finite())
        .map(|(name, value)| (name.as_str(), *value))
}

/// The first non-finite block output, if any: its name and its value.
///
/// Checked beside the MNA unknowns because a block output is not part of the solved state and
/// would otherwise go unexamined -- and a non-finite one is worse than a non-finite unknown,
/// since it reaches the circuit next step through a gate or a source value and makes the
/// solved state non-finite in turn. `sqrt` of a negative, `inv` of zero, `ln` of zero: the
/// library offers all three and none of them refuses.
///
/// A vector output names the element that broke, `NAME[i]`, because that is the name a rule
/// would have used to watch it.
pub fn first_non_finite_output(outputs: &BTreeMap<String, SignalValue>) -> Option<(String, f64)> {
    for (name, value) in outputs {
        match value {
            SignalValue::Scalar(v) if !v.is_finite() => return Some((name.clone(), *v)),
            SignalValue::Vector(values) => {
                if let Some((index, v)) = values.iter().enumerate().find(|(_, v)| !v.is_finite()) {
                    return Some((format!("{name}[{index}]"), *v));
                }
            }
            _ => {}
        }
    }
    None
}

/// The event a non-finite state produces.
///
/// `Severity::Fatal` and `Action::Stop`: continuing costs time and produces nothing, because
/// every later value is derived from this one.
pub fn event(name: &str, value: f64, step: usize, t: f64) -> EventRecord {
    EventRecord {
        time: t,
        step,
        step_time: t,
        rule: "finite".to_string(),
        target: name.to_string(),
        kind: EventKind::Instant,
        severity: Severity::Fatal,
        action: Action::Stop,
        value,
        limit: f64::INFINITY,
        duration: None,
        peak: None,
        peak_time: None,
        truncated: false,
        detail: Some(format!(
            "{name} is {}, so every later value is meaningless and every rule comparing one is \
             false. The run stops here.",
            describe(value)
        )),
        unit: String::new(),
    }
}

/// How to name a non-finite value in a report.
///
/// Distinguished because the three say different things about the cause: an infinity is an
/// overflow or a division by a conductance that went to zero, while a `NaN` is an
/// indeterminate form -- `0/0`, `inf - inf` -- usually one step later.
fn describe(value: f64) -> &'static str {
    if value.is_nan() {
        "not a number"
    } else if value > 0.0 {
        "positive infinity"
    } else {
        "negative infinity"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(pairs: &[(&str, f64)]) -> OperatingPoint {
        OperatingPoint {
            unknowns: pairs.iter().map(|(n, _)| (*n).to_string()).collect(),
            x: pairs.iter().map(|(_, v)| *v).collect(),
            diode_names: Vec::new(),
            diode_z: Vec::new(),
            diode_raw_ioff: Vec::new(),
        }
    }

    #[test]
    fn a_finite_state_is_not_reported() {
        let p = point(&[("V(a)", 1.0), ("I(L1)", -3.5), ("V(b)", 0.0)]);
        assert!(first_non_finite(&p).is_none());
    }

    #[test]
    fn the_first_non_finite_unknown_is_the_one_named() {
        // Two are broken; the first in MNA order is the one that says something about the
        // cause, since the others are usually derived from it within the same solve.
        let p = point(&[("V(a)", 1.0), ("V(b)", f64::NAN), ("I(L1)", f64::INFINITY)]);
        let (name, value) = first_non_finite(&p).expect("a finding");
        assert_eq!(name, "V(b)");
        assert!(value.is_nan());
    }

    #[test]
    fn an_infinity_is_caught_as_well_as_a_nan() {
        // `NaN > limit` is false and so is `inf > limit`... no: an infinity *does* compare, and
        // would trip a limit rule. It is still a dead run, and it is still caught here, because
        // the rule that would have tripped might not have been written.
        for broken in [f64::INFINITY, f64::NEG_INFINITY] {
            let p = point(&[("V(a)", broken)]);
            assert!(first_non_finite(&p).is_some(), "{broken}");
        }
    }

    #[test]
    fn a_non_finite_block_output_is_caught_too() {
        // It is not part of the solved state, so nothing else looks at it -- and it is the
        // worse case, because next step it reaches the circuit through a gate or a source.
        let outputs = BTreeMap::from([
            ("GOOD".to_string(), SignalValue::Scalar(1.0)),
            ("BAD".to_string(), SignalValue::Scalar(f64::NAN)),
        ]);
        let (name, value) = first_non_finite_output(&outputs).expect("a finding");
        assert_eq!(name, "BAD");
        assert!(value.is_nan());
    }

    #[test]
    fn a_vector_output_names_the_element_that_broke() {
        // `NAME[i]`, which is the name a rule would have used to watch it.
        let outputs = BTreeMap::from([(
            "IDQ".to_string(),
            SignalValue::Vector(vec![1.0, f64::INFINITY, 3.0]),
        )]);
        let (name, _) = first_non_finite_output(&outputs).expect("a finding");
        assert_eq!(name, "IDQ[1]");
    }

    #[test]
    fn healthy_outputs_are_not_reported() {
        let outputs = BTreeMap::from([
            ("A".to_string(), SignalValue::Scalar(0.0)),
            ("B".to_string(), SignalValue::Vector(vec![1.0, 2.0])),
        ]);
        assert!(first_non_finite_output(&outputs).is_none());
    }

    #[test]
    fn the_report_says_which_kind_of_non_finite_it_was() {
        assert!(event("V(a)", f64::NAN, 7, 1e-6)
            .detail
            .unwrap()
            .contains("not a number"));
        assert!(event("V(a)", f64::INFINITY, 7, 1e-6)
            .detail
            .unwrap()
            .contains("positive infinity"));
        assert!(event("V(a)", f64::NEG_INFINITY, 7, 1e-6)
            .detail
            .unwrap()
            .contains("negative infinity"));
    }

    #[test]
    fn the_event_is_fatal_and_stops_the_run() {
        let record = event("I(L1)", f64::NAN, 3, 2e-6);
        assert_eq!(record.severity, Severity::Fatal);
        assert_eq!(record.action, Action::Stop);
        assert_eq!(record.step, 3);
        assert_eq!(record.target, "I(L1)");
    }
}
