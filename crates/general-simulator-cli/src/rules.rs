//! `kind=rule` cards: parsing them out of the netlist, feeding them each step, and writing what
//! they found.
//!
//! # Why the cards are stripped before the solver sees them
//!
//! Exactly as `kind=measure` is (see [`crate::measure`]): `general-mna` has no `rule` entry in
//! its block dispatch and would reject the line. But there is a second reason here, and it is
//! the one that matters to anyone using checkpoints.
//!
//! A checkpoint refuses to load into a deck it did not come from, and the deck's identity is a
//! hash over the parsed statements. If a rule card were one of those statements, then editing a
//! limit -- raising a threshold, turning a `stop` into a `warning` -- would change the deck hash
//! and the checkpoint from the run that just stopped would no longer load. Stopping on a rule,
//! adjusting the rule and resuming is the whole point of having the two features together, so
//! the cards are removed here, before the hash is taken. Editing the *circuit* still invalidates
//! a checkpoint, which is correct: that state belongs to the old circuit.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;

use dae_runtime::{GateState, OperatingPoint, SignalValue};
use general_mna::block_graph::GateBinding;
use general_spice_core::ast::Statement;
use general_spice_core::Dialect;
use gsim_rules::{
    Action, Condition, EventRecord, Qualifiers, Rule, RuleSummary, Sample, Severity, Watch,
};

/// Pulls every `kind=rule` card out of `source`, returning the netlist without them.
pub fn extract(source: &str, dialect: Dialect) -> Result<(String, Vec<Rule>), String> {
    let statements = general_mna::parse_and_flatten(source, dialect)?;
    let mut rules = Vec::new();
    let mut blank_lines: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();

    for stmt in &statements {
        let Statement::BlockInstance(bi) = stmt else {
            continue;
        };
        let fields: BTreeMap<&str, &str> = bi
            .fields
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        if fields.get("kind").copied() != Some("rule") {
            continue;
        }
        rules.push(parse_rule(&bi.name, bi.span.start, &fields)?);
        for line in bi.span.clone() {
            blank_lines.insert(line);
        }
    }

    if blank_lines.is_empty() {
        return Ok((source.to_string(), rules));
    }
    let mut out = String::with_capacity(source.len());
    for (i, line) in source.lines().enumerate() {
        if !blank_lines.contains(&(i + 1)) {
            out.push_str(line);
        }
        out.push('\n');
    }
    Ok((out, rules))
}

fn parse_rule(name: &str, line: usize, fields: &BTreeMap<&str, &str>) -> Result<Rule, String> {
    let ctx = |msg: String| format!("line {line}: rule '{name}': {msg}");
    let rule_type = fields
        .get("type")
        .copied()
        .ok_or_else(|| ctx("missing field 'type'".into()))?;

    let severity = match fields.get("severity").copied().unwrap_or("warning") {
        "info" => Severity::Info,
        "warning" => Severity::Warning,
        "error" => Severity::Error,
        "fatal" => Severity::Fatal,
        other => {
            return Err(ctx(format!(
                "unknown severity '{other}' -- one of info, warning, error, fatal"
            )))
        }
    };
    // The spec's default per severity: anything at error or above is worth interrupting a
    // console session for; a warning is read afterwards.
    let default_action = if severity >= Severity::Error {
        "console"
    } else {
        "log"
    };
    let action = match fields.get("action").copied().unwrap_or(default_action) {
        "log" => Action::Log,
        "console" => Action::Console,
        "stop" => Action::Stop,
        "pause" => Action::Pause,
        other => {
            return Err(ctx(format!(
                "unknown action '{other}' -- one of log, console, stop, pause"
            )))
        }
    };

    let number = |key: &str| -> Result<f64, String> {
        let raw = fields
            .get(key)
            .copied()
            .ok_or_else(|| ctx(format!("missing field '{key}'")))?;
        parse_number(raw).ok_or_else(|| ctx(format!("field '{key}' is not a number: '{raw}'")))
    };
    let number_opt = |key: &str| -> Result<Option<f64>, String> {
        match fields.get(key).copied() {
            None => Ok(None),
            Some(raw) => Some(
                parse_number(raw)
                    .ok_or_else(|| ctx(format!("field '{key}' is not a number: '{raw}'"))),
            )
            .transpose(),
        }
    };
    let list = |key: &str| -> Result<Vec<String>, String> {
        let raw = fields
            .get(key)
            .copied()
            .ok_or_else(|| ctx(format!("missing field '{key}'")))?;
        let items: Vec<String> = raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        if items.is_empty() {
            return Err(ctx(format!("field '{key}' lists nothing")));
        }
        Ok(items)
    };

    let (watch, condition, target, unit) = match rule_type {
        "above" | "below" | "outside" | "inside" => {
            let quantity = fields
                .get("target")
                .copied()
                .ok_or_else(|| ctx("missing field 'target'".into()))?
                .to_string();
            let condition = match rule_type {
                "above" => Condition::Above {
                    limit: number("limit")?,
                },
                "below" => Condition::Below {
                    limit: number("limit")?,
                },
                "outside" => Condition::Outside {
                    low: number("low")?,
                    high: number("high")?,
                },
                _ => Condition::Inside {
                    low: number("low")?,
                    high: number("high")?,
                },
            };
            let unit = fields.get("unit").copied().unwrap_or("").to_string();
            (
                Watch::Quantity {
                    quantity: quantity.clone(),
                },
                condition,
                quantity,
                unit,
            )
        }
        "forbidden_state" => {
            let switches = list("switches")?;
            let max_on = match number_opt("max_on")? {
                Some(v) => v as usize,
                None => 1,
            };
            let target = fields
                .get("name")
                .copied()
                .map(str::to_string)
                .unwrap_or_else(|| switches.join("+"));
            (
                Watch::ForbiddenState {
                    switches,
                    max_on,
                    min_deadtime: number_opt("min_deadtime")?,
                },
                Condition::Above {
                    limit: max_on as f64,
                },
                target,
                " conducting".to_string(),
            )
        }
        "required_path" => {
            let switches = list("switches")?;
            let current = fields
                .get("current")
                .copied()
                .ok_or_else(|| ctx("missing field 'current'".into()))?
                .to_string();
            let i_min = number_opt("i_min")?.unwrap_or(0.0);
            let target = fields
                .get("name")
                .copied()
                .map(str::to_string)
                .unwrap_or_else(|| switches.join("+"));
            (
                Watch::RequiredPath {
                    switches,
                    current,
                    i_min,
                },
                Condition::Above { limit: i_min },
                target,
                fields.get("unit").copied().unwrap_or(" A").to_string(),
            )
        }
        other => {
            return Err(ctx(format!(
                "unknown rule type '{other}' -- one of above, below, outside, inside, \
                 forbidden_state, required_path"
            )))
        }
    };

    Ok(Rule {
        name: name.to_string(),
        target,
        watch,
        unit,
        condition,
        qualifiers: Qualifiers {
            min_duration: number_opt("hold")?,
            hysteresis: number_opt("hyst")?,
            after: number_opt("after")?,
            before: number_opt("before")?,
            max_events: number_opt("max_events")?.map(|v| v as usize),
            stop_after: number_opt("stop_after")?.map(|v| v as usize),
        },
        severity,
        action,
    })
}

/// A number in the netlist's own grammar, where `m` is milli and `MEG` is mega.
///
/// Written here rather than reused because `kind=measure` parses its fields with a plain
/// `f64::parse`, which silently rejects `200n` -- and a rule whose dead time has to be spelled
/// `0.0000002` is a rule nobody will write correctly.
pub fn parse_number(raw: &str) -> Option<f64> {
    let raw = raw.trim();
    if let Ok(value) = raw.parse::<f64>() {
        return Some(value);
    }
    // Longest suffix first: MEG must win over M, or mega silently becomes milli -- the one
    // confusion this grammar exists to prevent.
    //
    // The decimal suffixes are applied by rewriting the literal with an exponent and letting
    // Rust parse that, not by multiplying. `200.0 * 1e-9` is 2.0000000000000002e-7 while
    // `200e-9` is exactly 2e-7, and a dead time written `200n` has to be the same f64 as the
    // same number written any other way -- the project has already been bitten once by a
    // parameter that was not the value it was written as.
    const POWERS: &[(&str, i32)] = &[
        ("meg", 6),
        ("t", 12),
        ("g", 9),
        ("k", 3),
        ("m", -3),
        ("u", -6),
        ("n", -9),
        ("p", -12),
        ("f", -15),
    ];
    let lowered = raw.to_ascii_lowercase();
    // `mil` is 25.4 um -- not a power of ten, and checked before `m` would swallow it.
    if let Some(head) = lowered.strip_suffix("mil") {
        if let Ok(value) = head.parse::<f64>() {
            return Some(value * 25.4e-6);
        }
    }
    for (suffix, exponent) in POWERS {
        if let Some(head) = lowered.strip_suffix(suffix) {
            if head.parse::<f64>().is_ok() {
                if let Ok(value) = format!("{head}e{exponent}").parse::<f64>() {
                    return Some(value);
                }
            }
        }
    }
    None
}

/// One solver step, as the rules read it.
pub struct StepSample<'a> {
    pub point: &'a OperatingPoint,
    pub outputs: &'a BTreeMap<String, SignalValue>,
    pub gates: &'a BTreeMap<String, GateState>,
}

impl Sample for StepSample<'_> {
    fn get(&self, quantity: &str) -> Option<f64> {
        // The same names the waveform's own header carries, so a rule is written against what
        // the user already sees in the CSV rather than a second naming scheme.
        if let Some(value) = self.point.value(quantity) {
            return Some(value);
        }
        if let Some(SignalValue::Scalar(v)) = self.outputs.get(quantity) {
            return Some(*v);
        }
        // `NAME[i]` for one element of a vector-valued block output.
        let (name, index) = quantity.strip_suffix(']')?.split_once('[')?;
        let index: usize = index.parse().ok()?;
        match self.outputs.get(name) {
            Some(SignalValue::Vector(v)) => v.get(index).copied(),
            _ => None,
        }
    }

    fn switch_on(&self, switch: &str) -> Option<bool> {
        self.gates.get(switch).map(|state| *state == GateState::On)
    }
}

/// Resolves every gate the same way the solver's own loop does.
///
/// Recomputed here rather than passed out of the step callback: it keeps rules strictly
/// observers of the run, with no change to the solver's interface for their sake.
pub fn gate_states(
    gates: &BTreeMap<String, GateBinding>,
    outputs: &BTreeMap<String, SignalValue>,
) -> BTreeMap<String, GateState> {
    gates
        .iter()
        .filter(|(_, binding)| {
            binding
                .source_blocks()
                .iter()
                .flatten()
                .all(|name| outputs.contains_key(*name))
        })
        .map(|(name, binding)| (name.clone(), binding.resolve(outputs)))
        .collect()
}

/// Writes events where the spec says they go, as they happen.
pub struct Reporter {
    jsonl: Option<BufWriter<fs::File>>,
    text: Option<BufWriter<fs::File>>,
    console_at: Severity,
    pub worst: Option<Severity>,
    pub written: usize,
}

impl Reporter {
    /// `stem` is the netlist path; the two files land beside it as `<stem>.events.jsonl` and
    /// `<stem>.events.txt`, the same convention `kind=measure` uses for its `.log`.
    pub fn new(stem: &Path, console_at: Severity) -> Result<Reporter, String> {
        let jsonl_path = stem.with_extension("events.jsonl");
        let text_path = stem.with_extension("events.txt");
        let open = |path: &Path| -> Result<BufWriter<fs::File>, String> {
            fs::File::create(path)
                .map(BufWriter::new)
                .map_err(|e| format!("creating {}: {e}", path.display()))
        };
        Ok(Reporter {
            jsonl: Some(open(&jsonl_path)?),
            text: Some(open(&text_path)?),
            console_at,
            worst: None,
            written: 0,
        })
    }

    pub fn record(&mut self, event: &EventRecord) -> Result<(), String> {
        self.worst = Some(match self.worst {
            Some(worst) => worst.max(event.severity),
            None => event.severity,
        });
        self.written += 1;
        if let Some(writer) = &mut self.jsonl {
            writeln!(writer, "{}", event.to_json_line())
                .map_err(|e| format!("writing the event log: {e}"))?;
        }
        let line = event.console_line();
        if let Some(writer) = &mut self.text {
            writeln!(writer, "{line}").map_err(|e| format!("writing the event log: {e}"))?;
        }
        // `log` keeps to the files; everything else earns the console, as does any event at or
        // above the console threshold whatever its own action says.
        if event.action != Action::Log || event.severity >= self.console_at {
            eprintln!("{line}");
        }
        Ok(())
    }

    pub fn finish(&mut self, summaries: &[RuleSummary]) -> Result<(), String> {
        let report = summary_report(summaries);
        if let Some(writer) = &mut self.text {
            writeln!(writer, "\n{report}").map_err(|e| format!("writing the event log: {e}"))?;
            writer
                .flush()
                .map_err(|e| format!("writing the event log: {e}"))?;
        }
        if let Some(writer) = &mut self.jsonl {
            for summary in summaries {
                let line = serde_json::to_string(summary)
                    .map_err(|e| format!("writing the event log: {e}"))?;
                writeln!(writer, "{line}").map_err(|e| format!("writing the event log: {e}"))?;
            }
            writer
                .flush()
                .map_err(|e| format!("writing the event log: {e}"))?;
        }
        eprintln!("{report}");
        Ok(())
    }
}

/// The end-of-run table: what every rule saw, including the ones that saw nothing.
pub fn summary_report(summaries: &[RuleSummary]) -> String {
    if summaries.is_empty() {
        return String::new();
    }
    let mut out = String::from("--- rules ---\n");
    for summary in summaries {
        out.push_str(&summary.line());
        out.push('\n');
    }
    let violated = summaries
        .iter()
        .filter(|s| s.entries > 0 || s.instants > 0)
        .count();
    // "Not evaluated" means the rule could not read what it watches -- not that it had
    // something to say. A rule with dead-time findings and no interval violations belongs in
    // the violated column, not this one.
    let blind = summaries
        .iter()
        .filter(|s| s.missing_samples > 0 && s.entries == 0 && s.instants == 0)
        .count();
    out.push_str(&format!(
        "{} rule(s): {} violated, {} passed",
        summaries.len(),
        violated,
        summaries.len() - violated - blind,
    ));
    if blind > 0 {
        out.push_str(&format!(", {blind} not evaluated"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_follow_the_netlists_own_grammar() {
        assert_eq!(parse_number("50"), Some(50.0));
        assert_eq!(parse_number("1.5e3"), Some(1500.0));
        // Bit-exact, not merely close: `200n` must be the same f64 as `200e-9`, which rules
        // out applying the suffix by multiplication.
        assert_eq!(parse_number("200n").unwrap().to_bits(), 200e-9f64.to_bits());
        assert_eq!(parse_number("200n"), Some(2e-7));
        assert_eq!(parse_number("36u"), Some(36e-6));
        assert_eq!(parse_number("20m"), Some(20e-3));
        assert_eq!(parse_number("31.25k"), Some(31_250.0));
        assert_eq!(parse_number("x"), None);
    }

    #[test]
    fn meg_is_mega_and_m_is_milli() {
        // The confusion the grammar exists to prevent, and the reason the suffix table is
        // ordered longest-first rather than by magnitude.
        assert_eq!(parse_number("2MEG"), Some(2e6));
        assert_eq!(parse_number("2meg"), Some(2e6));
        assert_eq!(parse_number("2M"), Some(2e-3));
        assert_eq!(parse_number("2m"), Some(2e-3));
    }
}
