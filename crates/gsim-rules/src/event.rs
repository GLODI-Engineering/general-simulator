//! The event record, and the vocabulary around it.
//!
//! One record shape feeds every output -- the JSONL log, the console line, the end-of-run
//! summary, and later the viewer's markers -- so those four can never disagree about what
//! happened. That is the whole reason the record is a type here rather than a format string at
//! each output: the log said one thing and the summary another in every tool this replaces.

use serde::{Deserialize, Serialize};

/// How bad a violation is, independent of what is then done about it.
///
/// Severity and action are deliberately separate. The same over-current is a `warning` someone
/// reads afterwards on a first exploratory run and a `fatal` that stops a regression run, and
/// the rule should not have to be rewritten between the two.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
    Fatal,
}

impl Severity {
    /// The label a console line carries, padded to a common width so columns line up.
    pub fn label(self) -> &'static str {
        match self {
            Severity::Info => "INFO ",
            Severity::Warning => "WARN ",
            Severity::Error => "ERROR",
            Severity::Fatal => "FATAL",
        }
    }
}

/// What happens when a rule fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    /// The log file only.
    Log,
    /// The log file and the console.
    Console,
    /// Log, console, then end the run cleanly -- finishing the current step, flushing every
    /// waveform and log file, writing the summary. The results up to the stop stay readable.
    Stop,
    /// GUI only: hold at the event with the state intact so it can be inspected and resumed.
    /// Accepted and recorded here; a batch run treats it as [`Action::Console`], because a
    /// command-line run has nobody to resume it and silently stopping would be worse.
    Pause,
}

/// Which moment in a violation's life the record describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    /// The condition became true.
    Enter,
    /// The condition became false. Carries the duration, the worst value reached and when.
    Leave,
    /// A condition with no duration -- it was true at one instant and the question of when it
    /// stops does not arise.
    Instant,
    /// Written once per rule at the end of the run.
    Summary,
}

/// One thing that happened, in the form every output reads.
///
/// `value` and `limit` are what the rule compared; on a [`EventKind::Leave`] the extra three
/// fields describe the interval that just ended. They are `None` on an enter because they are
/// not known yet -- which is the honest encoding, and why a leave is a separate record rather
/// than a mutation of the enter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventRecord {
    /// Simulation time, interpolated to where the quantity crossed the limit inside the step --
    /// not the end of the step in which the crossing was noticed. A 10 ns step would otherwise
    /// quantise every reported time to 10 ns, which is the same order as the dead times these
    /// rules exist to measure.
    pub time: f64,
    /// The solver step the crossing was noticed in. Kept beside the interpolated time because
    /// reproducing a problem means re-running to a step, not to a float.
    pub step: usize,
    /// The solver's own time at the end of that step, before interpolation. Logged alongside
    /// `time` so a reader can tell a crossing placed inside a step from one that happened to
    /// land on a step boundary, and so the record can be lined up against a waveform file whose
    /// rows sit on those boundaries.
    pub step_time: f64,
    pub rule: String,
    /// The component, switch or signal the rule watches.
    pub target: String,
    pub kind: EventKind,
    pub severity: Severity,
    /// What was actually done, which is not always what the rule asked for: a `pause` in a
    /// batch run is recorded as the `console` it became.
    pub action: Action,
    pub value: f64,
    pub limit: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak_time: Option<f64>,
    /// Set on a leave the end of the run forced: the condition was still true when the run
    /// ended, so the duration is a lower bound rather than a measurement.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    /// Which components are responsible, when the rule spans several. "Both switches of the
    /// leg conducted" is unactionable; naming the two is what sends the reader to the right
    /// gate signal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The unit `value` and `limit` are in.
    ///
    /// Carried on the record rather than supplied by whoever prints it, because one rule can
    /// report in two units: a leg rule's violation is a count of switches, and its dead-time
    /// finding is a time. Printing the dead time in the violation's unit produced
    /// `0.0000 conducting`, which is how this field came to exist.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub unit: String,
}

impl EventRecord {
    /// The record as one line of `events.jsonl`.
    ///
    /// The log is one JSON object per line so a tool can stream it and a person can still read
    /// it; this lives beside the record rather than at each writer so the log, the viewer and
    /// anything reading it later cannot drift into three spellings of the same event.
    pub fn to_json_line(&self) -> String {
        // The struct derives Serialize over plain fields, so this cannot fail for any value
        // the engine builds; a NaN would, and a NaN here is a bug worth seeing rather than
        // swallowing.
        serde_json::to_string(self)
            .unwrap_or_else(|e| format!("{{\"error\":\"event could not be serialised: {e}\"}}"))
    }

    /// One line for the console, in the spec's own shape:
    /// `[  2.3140 ms] WARN  L1.i_max       enter  i=52.3 A > 50 A`.
    ///
    /// The unit is the caller's, since this crate never learns what a quantity means.
    pub fn console_line(&self) -> String {
        let unit = self.unit.as_str();
        // A value already in seconds is read as a duration, not as a bare float.
        let quantity = |value: f64| -> String {
            if unit == "s" {
                format_time(value)
            } else {
                format!("{value:.4}{unit}")
            }
        };
        let kind = match self.kind {
            EventKind::Enter => "enter",
            EventKind::Leave => "leave",
            EventKind::Instant => "at",
            EventKind::Summary => "total",
        };
        let mut line = format!(
            "[{:>9}] {} {:<18} {:<6} {}",
            format_time(self.time),
            self.severity.label(),
            format!("{}.{}", self.target, self.rule),
            kind,
            quantity(self.value),
        );
        if let (Some(duration), Some(peak)) = (self.duration, self.peak) {
            line.push_str(&format!(
                "  for {} (peak {}{})",
                format_time(duration),
                quantity(peak),
                if self.truncated { ", run ended" } else { "" }
            ));
        } else {
            line.push_str(&format!("  limit {}", quantity(self.limit)));
        }
        if let Some(detail) = &self.detail {
            line.push_str(&format!("  [{detail}]"));
        }
        line
    }
}

/// A time in the unit that keeps it readable, which for power electronics is rarely seconds.
///
/// A dead time printed as `0.000000200 s` is read wrong at a glance; `200.0000 ns` is not.
pub fn format_time(seconds: f64) -> String {
    let magnitude = seconds.abs();
    if magnitude == 0.0 {
        return "0 s".to_string();
    }
    let (scale, unit) = if magnitude < 1e-6 {
        (1e9, "ns")
    } else if magnitude < 1e-3 {
        (1e6, "us")
    } else if magnitude < 1.0 {
        (1e3, "ms")
    } else {
        (1.0, "s")
    };
    format!("{:.4} {unit}", seconds * scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severities_order_by_how_bad_they_are() {
        // The end-of-run colouring and `--fail-on` both rank severities, so the derived Ord has
        // to match the reading order rather than the declaration order by accident.
        assert!(Severity::Info < Severity::Warning);
        assert!(Severity::Warning < Severity::Error);
        assert!(Severity::Error < Severity::Fatal);
    }

    #[test]
    fn time_is_printed_in_the_unit_that_keeps_it_readable() {
        assert_eq!(format_time(0.0), "0 s");
        assert_eq!(format_time(200e-9), "200.0000 ns");
        assert_eq!(format_time(2.5e-6), "2.5000 us");
        assert_eq!(format_time(2.314e-3), "2.3140 ms");
        assert_eq!(format_time(1.5), "1.5000 s");
        // Negative durations never occur, but a negative *time* offset might once rules can be
        // anchored to an event rather than to zero.
        assert_eq!(format_time(-2.5e-6), "-2.5000 us");
    }

    #[test]
    fn an_enter_line_names_the_limit_and_a_leave_line_the_duration() {
        let enter = EventRecord {
            time: 2.314e-3,
            step: 231,
            step_time: 2.32e-3,
            rule: "i_max".into(),
            target: "L1".into(),
            kind: EventKind::Enter,
            severity: Severity::Warning,
            action: Action::Console,
            value: 52.3,
            limit: 50.0,
            duration: None,
            peak: None,
            peak_time: None,
            truncated: false,
            detail: None,
            unit: " A".into(),
        };
        let line = enter.console_line();
        assert!(line.contains("2.3140 ms"), "{line}");
        assert!(line.contains("WARN"), "{line}");
        assert!(line.contains("L1.i_max"), "{line}");
        assert!(line.contains("enter"), "{line}");
        assert!(line.contains("limit 50.0000 A"), "{line}");

        let leave = EventRecord {
            kind: EventKind::Leave,
            duration: Some(0.688e-3),
            peak: Some(61.0),
            peak_time: Some(2.9e-3),
            ..enter
        };
        let line = leave.console_line();
        assert!(line.contains("for 688.0000 us"), "{line}");
        assert!(line.contains("peak 61.0000 A"), "{line}");
    }

    #[test]
    fn a_truncated_leave_says_the_run_ended_rather_than_the_condition() {
        let record = EventRecord {
            time: 5e-3,
            step: 500,
            step_time: 5e-3,
            rule: "i_max".into(),
            target: "L1".into(),
            kind: EventKind::Leave,
            severity: Severity::Warning,
            action: Action::Log,
            value: 52.0,
            limit: 50.0,
            duration: Some(1e-3),
            peak: Some(52.0),
            peak_time: Some(4.5e-3),
            truncated: true,
            detail: None,
            unit: " A".into(),
        };
        assert!(record.console_line().contains("run ended"));
    }

    #[test]
    fn the_optional_fields_stay_out_of_the_json_when_absent() {
        // The log is one JSON object per line and gets read by eye as often as by a tool; an
        // enter carrying `"duration": null` invites the reader to believe it has one.
        let enter = EventRecord {
            time: 1.0,
            step: 1,
            step_time: 1.0,
            rule: "r".into(),
            target: "t".into(),
            kind: EventKind::Enter,
            severity: Severity::Info,
            action: Action::Log,
            value: 1.0,
            limit: 0.5,
            duration: None,
            peak: None,
            peak_time: None,
            truncated: false,
            detail: None,
            unit: " A".into(),
        };
        let json = serde_json::to_string(&enter).unwrap();
        assert!(!json.contains("duration"), "{json}");
        assert!(!json.contains("truncated"), "{json}");
        assert!(json.contains("\"kind\":\"enter\""), "{json}");
        assert!(json.contains("\"severity\":\"info\""), "{json}");
    }
}
