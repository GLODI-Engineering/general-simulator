//! What each rule has to say once the run is over.
//!
//! The spec calls this "the part a reviewer reads first", and that sets its shape: a rule that
//! never fired is listed as passed rather than omitted. Silence is ambiguous -- a rule that was
//! never violated and a rule that was never evaluated look identical in a log that only records
//! violations -- and the ambiguity is the dangerous kind, because the reader concludes the
//! circuit is fine.

use serde::{Deserialize, Serialize};

use crate::event::{format_time, Severity};

/// One rule's account of the whole run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuleSummary {
    pub rule: String,
    pub target: String,
    pub severity: Severity,
    pub unit: String,
    /// Completed violations that were reported.
    pub entries: usize,
    /// Excursions the hold time discarded. A large number here means the hold time is hiding
    /// something; without it the reader would have to guess whether the rule was quiet or deaf.
    pub filtered: usize,
    /// Records that `max_events` kept out of the log. The count still lands here, so the budget
    /// never loses information, only detail.
    pub suppressed: usize,
    pub total_time: f64,
    pub worst_value: Option<f64>,
    pub worst_time: Option<f64>,
    /// Steps where the rule's quantity was not in the sample. Anything above zero means the
    /// rule was partly or wholly blind, which is not the same as passing.
    pub missing_samples: usize,
    /// Findings that have no duration, such as a dead time measured at a transition. They are
    /// violations too -- a summary that called a rule "passed" while it had raised four
    /// dead-time warnings would be worse than no summary.
    #[serde(default)]
    pub instants: usize,
    /// Shortest gap measured between one switch of a group opening and the next closing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shortest_deadtime: Option<f64>,
}

impl RuleSummary {
    /// Whether this rule has nothing to report -- and was in a position to know.
    pub fn passed(&self) -> bool {
        self.entries == 0 && self.instants == 0 && self.missing_samples == 0
    }

    /// One line of the end-of-run table.
    pub fn line(&self) -> String {
        let name = format!("{}.{}", self.target, self.rule);
        if self.missing_samples > 0 && self.entries == 0 {
            return format!(
                "  {name:<28} NOT EVALUATED  ({} step(s) had no such quantity)",
                self.missing_samples
            );
        }
        if self.entries == 0 && self.instants > 0 {
            let mut line = format!(
                "  {name:<28} {:<5} {:>3} finding(s)",
                self.severity.label(),
                self.instants
            );
            if let Some(gap) = self.shortest_deadtime {
                line.push_str(&format!(", shortest dead time {}", format_time(gap)));
            }
            return line;
        }
        if self.entries == 0 {
            let mut line = format!("  {name:<28} passed");
            if self.filtered > 0 {
                line.push_str(&format!(
                    "  ({} excursion(s) shorter than the hold time)",
                    self.filtered
                ));
            }
            if let Some(gap) = self.shortest_deadtime {
                line.push_str(&format!("  shortest dead time {}", format_time(gap)));
            }
            return line;
        }
        let mut line = format!(
            "  {name:<28} {:<5} {:>3} time(s), {} total",
            self.severity.label(),
            self.entries,
            format_time(self.total_time),
        );
        if let (Some(value), Some(time)) = (self.worst_value, self.worst_time) {
            line.push_str(&format!(
                ", worst {}{} at {}",
                crate::format_magnitude(value),
                self.unit,
                format_time(time)
            ));
        }
        if self.filtered > 0 {
            line.push_str(&format!(", {} filtered", self.filtered));
        }
        if self.suppressed > 0 {
            line.push_str(&format!(", {} not logged", self.suppressed));
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary() -> RuleSummary {
        RuleSummary {
            rule: "i_max".into(),
            target: "L1".into(),
            severity: Severity::Warning,
            unit: " A".into(),
            entries: 0,
            filtered: 0,
            suppressed: 0,
            total_time: 0.0,
            worst_value: None,
            worst_time: None,
            missing_samples: 0,
            instants: 0,
            shortest_deadtime: None,
        }
    }

    #[test]
    fn a_rule_that_never_fired_is_reported_as_passed() {
        assert!(summary().passed());
        assert!(summary().line().contains("passed"));
    }

    #[test]
    fn a_rule_that_could_not_read_its_quantity_is_not_passed() {
        // The distinction the whole summary exists for: quiet because nothing went wrong, or
        // quiet because it was never looking.
        let blind = RuleSummary {
            missing_samples: 1200,
            ..summary()
        };
        assert!(!blind.passed());
        let line = blind.line();
        assert!(line.contains("NOT EVALUATED"), "{line}");
        assert!(!line.contains("passed"), "{line}");
    }

    #[test]
    fn a_violated_rule_reports_its_count_total_and_worst() {
        let fired = RuleSummary {
            entries: 3,
            total_time: 1.2e-3,
            worst_value: Some(61.5),
            worst_time: Some(2.9e-3),
            ..summary()
        };
        let line = fired.line();
        assert!(line.contains("3 time(s)"), "{line}");
        assert!(line.contains("1.2000 ms total"), "{line}");
        assert!(line.contains("worst 61.5000 A at 2.9000 ms"), "{line}");
    }

    #[test]
    fn filtered_and_suppressed_counts_are_shown_rather_than_lost() {
        let noisy = RuleSummary {
            entries: 2,
            filtered: 97,
            suppressed: 40,
            total_time: 1e-6,
            worst_value: Some(1.0),
            worst_time: Some(1e-6),
            ..summary()
        };
        let line = noisy.line();
        assert!(line.contains("97 filtered"), "{line}");
        assert!(line.contains("40 not logged"), "{line}");
    }
}
