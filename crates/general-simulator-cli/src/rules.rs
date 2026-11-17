//! `kind=rule` cards and ratings written on a device line: parsing them out of the netlist,
//! feeding them each step, and writing what they found.
//!
//! The specification offers two ways to write the same thing, and both compile to the same
//! [`Rule`] objects. A card says everything explicitly; a rating says it where the component is
//! declared, which is where a datasheet number belongs:
//!
//! ```text
//! Ls a b 36u i_max=60 i_sat=70 on_violation=console
//! ```
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
//!
//! A rating is stripped for the same reason, which costs more work: the rest of the device line
//! has to survive, so the rating tokens are cut out of the line rather than the line being
//! blanked. Without that, raising `i_max` -- the most natural thing to do after a run stops on
//! it -- would change the deck hash and refuse the checkpoint of the run that just stopped.
//! `general-mna` accepts these fields in their own right (it carries any field a caller
//! registers through `BuildOptions::annotate`, see that repository's #25), so a consumer that
//! registers [`RATING_FIELDS`] -- a GUI, or anything handing the netlist straight to the
//! builder -- can keep them instead of being refused the deck. This crate strips them anyway,
//! because the hash is what makes stop-adjust-resume work.

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufWriter, Write};
use std::path::Path;

use dae_runtime::{GateState, OperatingPoint, SignalValue};
use general_mna::block_graph::GateBinding;
use general_spice_core::ast::{ElementInstance, Statement};
use general_spice_core::Dialect;
use gsim_rules::{
    Action, Condition, EventRecord, PeriodBoundary, PeriodMeasure, Qualifiers, Rule, RuleSummary,
    Sample, Severity, Watch,
};

/// A declared power port: a voltage measured across one node pair, a current measured through
/// one branch, and therefore a power.
///
/// ## Why this exists at all
///
/// The rules engine reads *named scalars*. Power is $p = v \cdot i$, a product of two names, and
/// the engine has no arithmetic -- so there was nothing to write in a rule's `target=`. Three
/// ways out were available: let the netlist compute it with a `kind=product` chain per port,
/// give the rule grammar an expression language, or make the port a declared object. The port
/// object was chosen because loss estimation needs exactly the same thing: power in minus power
/// out, per element and per stage. An expression evaluator would have been re-derived into
/// ports anyway, and it would have turned every `target=` field into a parser.
///
/// ## Sign convention: the plain product, and an explicit minus where one is needed
///
/// $p = V(\text{positive}, \text{negative}) \times I(\text{branch})$ -- the plain product, with
/// the solver's own sign for the branch current and no hidden negation anywhere. `i=` may carry
/// a leading minus (`i=-VIN`) which negates that current, and that is the only sign control
/// there is.
///
/// A single built-in negation cannot serve both ends of a converter, which was established by
/// measurement rather than argument. The solver reports a branch current in the passive
/// convention, positive *into* the element's first node:
///
/// - On a **source**, current leaves the positive terminal: `VIN in 0 12` feeding `R1 in 0 12`
///   solves to `I(VIN) = -1 A` while 1 A plainly flows from `in` through the resistor. So an
///   input port reads positive with the minus: `P_IN kind=port v=(in,0) i=-VIN` gives
///   $12 \times 1 = +12$ W.
/// - On a **series sense element**, current enters the positive terminal: with
///   `VSENSE out load 0` in line with the load, `I(VSENSE)` is already the load current. So an
///   output port reads positive without one: `P_OUT kind=port v=(out,0) i=VSENSE` gives
///   $V_\text{out} \times I_\text{load}$.
///
/// A first version of this type negated unconditionally, on the strength of the source
/// measurement alone. The output port of a buck then came out negative and its rule silently
/// never fired -- caught by probing a real converter, which is why both canonical declarations
/// are written out above and asserted by a test.
///
/// Those two lines are the whole vocabulary: a port on a source carries the minus, a port on a
/// sense element does not. A port declared the wrong way round reports a negative power, so an
/// `type=efficiency` rule with `min=0` reports the mistake rather than inverting the answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Port {
    /// The port's own name, which is what a rule's `target=`/`in=`/`out=` names.
    pub name: String,
    /// The node the voltage is measured from. `0` is ground.
    pub positive: String,
    /// The node the voltage is measured to. `0` is ground.
    pub negative: String,
    /// The element whose branch current is this port's current. Must be one the solver actually
    /// solves a current for -- `V`, `L`, `E` or `H` -- which is why a sense source is the usual
    /// way to put a port anywhere else.
    pub branch: String,
    /// Whether `i=` carried a leading minus, as a port on a source does.
    pub negate: bool,
}

/// A switch's channel, as a conduction loss needs it: the two nodes its voltage is across and
/// the resistance it is stamped as while commanded on.
///
/// Collected from two statements that share a name -- the device line
/// (`DSW in mid idealswitchmodel`) carries the node pair, and the `kind=ideal_switch` line
/// carries `r_on`. Neither is restated on a rule card, because an `r_on` written twice can
/// drift from the device's and would then report a loss the circuit does not have.
#[derive(Debug, Clone, PartialEq)]
pub struct SwitchChannel {
    pub positive: String,
    pub negative: String,
    pub r_on: f64,
}

/// What a deck carried besides the circuit: rules, ports, and the netlist without them.
#[derive(Debug)]
pub struct Extracted {
    /// The netlist with every card and rating removed, which is what the solver parses.
    pub netlist: String,
    pub rules: Vec<Rule>,
    /// Declared ports, by name.
    pub ports: BTreeMap<String, Port>,
    /// Every `kind=ideal_switch`'s channel, so `P_cond(NAME)` can be resolved. Read, never
    /// stripped: the solver needs those lines.
    pub switches: BTreeMap<String, SwitchChannel>,
}

/// Parses one `kind=port` declaration.
fn parse_port(name: &str, line: usize, fields: &BTreeMap<&str, &str>) -> Result<Port, String> {
    let ctx = |msg: String| format!("line {line}: port '{name}': {msg}");
    let pair = fields.get("v").copied().ok_or_else(|| {
        ctx("missing field 'v' -- the node pair the voltage is across, v=(p,n)".into())
    })?;
    let inner = pair
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or_else(|| {
            ctx(format!(
                "field 'v' must be a node pair like v=(out,0), not '{pair}'"
            ))
        })?;
    let (positive, negative) = inner.split_once(',').ok_or_else(|| {
        ctx(format!(
            "field 'v' needs two nodes separated by a comma, not '{pair}'"
        ))
    })?;
    if positive.trim().is_empty() || negative.trim().is_empty() {
        return Err(ctx(format!("field 'v' has an empty node: '{pair}'")));
    }
    let branch = fields
        .get("i")
        .copied()
        .ok_or_else(|| {
            ctx(
                "missing field 'i' -- the element whose branch current is this port's current \
                 (a V, L, E or H; put a zero-volt sense source in series to measure anywhere \
                 else)"
                    .into(),
            )
        })?
        .trim()
        .to_string();
    // A leading minus negates the branch current, which is what a port on a source needs. It is
    // the only sign control, and it is visible in the deck rather than implied by the kind of
    // element named.
    let (negate, branch) = match branch.strip_prefix('-') {
        Some(rest) => (true, rest.trim().to_string()),
        None => (false, branch),
    };
    if branch.is_empty() {
        return Err(ctx("field 'i' names no element".into()));
    }
    Ok(Port {
        name: name.to_string(),
        positive: positive.trim().to_string(),
        negative: negative.trim().to_string(),
        branch,
        negate,
    })
}

/// Pulls every `kind=rule` card and every device-line rating out of `source`, returning the
/// netlist without them.
///
/// A card's whole line goes; a rating is cut token by token, because the device it is written on
/// still has to be there afterwards. See the module doc for why either is removed at all.
pub fn extract(source: &str, dialect: Dialect) -> Result<Extracted, String> {
    let statements = general_mna::parse_and_flatten(source, dialect)?;
    let mut rules = Vec::new();
    let mut ports: BTreeMap<String, Port> = BTreeMap::new();
    // A switch's channel is assembled from two statements sharing a name, which may appear in
    // either order, so each half is collected separately and joined at the end.
    let mut switch_nodes: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut switch_r_on: BTreeMap<String, f64> = BTreeMap::new();
    let mut blank_lines: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
    // Rating tokens to cut, keyed by the line they sit on. A device card may span continuation
    // lines, so the token's own line is what matters, not the element's first.
    let mut cut_tokens: BTreeMap<usize, Vec<String>> = BTreeMap::new();

    for stmt in &statements {
        if let Statement::ElementInstance(element) = stmt {
            if let [positive, negative] = element.nodes.as_slice() {
                switch_nodes.insert(element.name.clone(), (positive.clone(), negative.clone()));
            }
            let (element_rules, cuts) = ratings_of(element)?;
            rules.extend(element_rules);
            for token in cuts {
                // Which line the token is on is not recorded by the parser, so it is found by
                // looking: a token is cut from the first line of the element's span that still
                // contains it, which is the line it was written on.
                for line in element.span.clone() {
                    let already = cut_tokens.entry(line).or_default();
                    let text = source.lines().nth(line - 1).unwrap_or("");
                    if contains_token(text, &token) && !already.contains(&token) {
                        already.push(token.clone());
                        break;
                    }
                }
            }
            continue;
        }
        let Statement::BlockInstance(bi) = stmt else {
            continue;
        };
        let fields: BTreeMap<&str, &str> = bi
            .fields
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        match fields.get("kind").copied() {
            // A port is stripped for the same two reasons a rule card is: the solver would
            // reject it as an unknown device kind, and it must not reach the parsed statements
            // the checkpoint's deck hash is taken over -- adding a port to inspect a stopped
            // run would otherwise invalidate the checkpoint of the run being inspected.
            Some("port") => {
                let port = parse_port(&bi.name, bi.span.start, &fields)?;
                if let Some(first) = ports.insert(port.name.clone(), port) {
                    return Err(format!(
                        "line {}: port '{}' is declared twice -- two ports of one name would                          make a rule naming it ambiguous",
                        bi.span.start, first.name
                    ));
                }
            }
            Some("rule") => rules.push(parse_rule(&bi.name, bi.span.start, &fields)?),
            // Read, not stripped: the solver needs this line, and `continue` leaves it in place.
            Some("ideal_switch") => {
                if let Some(r_on) = fields.get("r_on").and_then(|raw| parse_number(raw)) {
                    switch_r_on.insert(bi.name.clone(), r_on);
                }
                continue;
            }
            _ => continue,
        }
        for line in bi.span.clone() {
            blank_lines.insert(line);
        }
    }

    // Only a name with both halves is a channel: a `kind=ideal_switch` whose device line has
    // other than two nodes has no single voltage to be across, and a two-node element that is
    // not a switch has no `r_on`.
    let switches: BTreeMap<String, SwitchChannel> = switch_r_on
        .into_iter()
        .filter_map(|(name, r_on)| {
            let (positive, negative) = switch_nodes.get(&name)?.clone();
            Some((
                name,
                SwitchChannel {
                    positive,
                    negative,
                    r_on,
                },
            ))
        })
        .collect();

    if blank_lines.is_empty() && cut_tokens.is_empty() {
        return Ok(Extracted {
            netlist: source.to_string(),
            rules,
            ports,
            switches,
        });
    }
    let mut out = String::with_capacity(source.len());
    for (i, line) in source.lines().enumerate() {
        let number = i + 1;
        if !blank_lines.contains(&number) {
            match cut_tokens.get(&number) {
                None => out.push_str(line),
                Some(tokens) => out.push_str(&without_tokens(line, tokens)),
            }
        }
        out.push('\n');
    }
    Ok(Extracted {
        netlist: out,
        rules,
        ports,
        switches,
    })
}

/// Whether `line` carries `token` as a whole whitespace-delimited token.
///
/// Substring matching would be wrong twice over: `i_max=6` occurs inside `i_max=60`, and a
/// node named `i_max=60` cannot exist but a value like `60` certainly occurs elsewhere.
fn contains_token(line: &str, token: &str) -> bool {
    line.split_whitespace().any(|found| found == token)
}

/// `line` with each of `tokens` removed once, leaving the rest of it byte-for-byte.
fn without_tokens(line: &str, tokens: &[String]) -> String {
    let mut remaining: Vec<&String> = tokens.iter().collect();
    let kept: Vec<&str> = line
        .split_whitespace()
        .filter(|found| {
            if let Some(index) = remaining.iter().position(|token| token.as_str() == *found) {
                remaining.remove(index);
                return false;
            }
            true
        })
        .collect();
    // Leading whitespace is not significant on a SPICE device line, but a leading `+` is: it
    // marks a continuation, and dropping the indentation of one would join two cards.
    let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
    format!("{indent}{}", kept.join(" "))
}

/// Every field name this crate claims on a device line.
///
/// Registered with `general-mna` through `BuildOptions::annotate` so a deck carrying them is
/// valid for any consumer that opts in, rather than only for this binary. Exposed for that
/// registration and for a test that keeps the two lists from drifting apart.
pub const RATING_FIELDS: &[&str] = &[
    "i_max",
    "i_sat",
    "v_max",
    "on_violation",
    "severity",
    "hold",
];

/// The rules a device line's ratings compile to, and the tokens to cut out of the line.
///
/// A rating is a limit written where the component is declared, which is where a datasheet
/// number belongs. It produces exactly the [`Rule`] a card would, so everything downstream --
/// hold times, the event log, the summary table, `stop` -- behaves identically whichever way
/// the limit was written.
///
/// **A rating is a limit on magnitude, not on signed value.** `i_max=60` compiles to
/// `outside(-60, +60)`, not to `above(60)`. A buck inductor's current is one-signed and either
/// reading would do; a bridge inductor's is not, and `above` would watch a 70 A reverse
/// current go by without a word. A rating that only half-works is worse than none, because it
/// reads as covered.
fn ratings_of(element: &ElementInstance) -> Result<(Vec<Rule>, Vec<String>), String> {
    let line = element.span.start;
    let name = &element.name;
    let ctx = |msg: String| format!("line {line}: device '{name}': {msg}");

    let mut written: Vec<(String, String, String)> = Vec::new();
    for token in &element.raw_params {
        let Some((key, value)) = token.split_once('=') else {
            continue;
        };
        let key = key.trim().to_ascii_lowercase();
        if RATING_FIELDS.contains(&key.as_str()) {
            written.push((key, value.to_string(), token.clone()));
        }
    }
    if written.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }

    // `on_violation`, `severity` and `hold` modify a rating; alone they modify nothing. Left
    // accepted, the line would be stripped and the field would silently do nothing, which is
    // the failure this feature exists to remove -- and `on_violation=stop` on its own reads
    // exactly like a deck that is guarded.
    if written
        .iter()
        .all(|(key, _, _)| matches!(key.as_str(), "on_violation" | "severity" | "hold"))
    {
        let names: Vec<&str> = written.iter().map(|(key, _, _)| key.as_str()).collect();
        return Err(ctx(format!(
            "'{}' modifies a rating, and this device declares none -- add one of \
             i_max, i_sat or v_max, or remove the field",
            names.join("', '")
        )));
    }

    let cuts: Vec<String> = written.iter().map(|(_, _, token)| token.clone()).collect();
    let field = |wanted: &str| -> Option<&str> {
        written
            .iter()
            .find(|(key, _, _)| key == wanted)
            .map(|(_, value, _)| value.as_str())
    };

    // A rating is a hard limit, so it defaults to `error` where a card's `type=above` defaults
    // to `warning`: the number came off a datasheet, and exceeding it is not a matter of taste.
    // The action then follows from the severity exactly as it does for a card.
    let severity = match field("severity").unwrap_or("error") {
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
    let default_action = if severity >= Severity::Error {
        "console"
    } else {
        "log"
    };
    let action = match field("on_violation").unwrap_or(default_action) {
        "log" => Action::Log,
        "console" => Action::Console,
        "stop" => Action::Stop,
        "pause" => Action::Pause,
        other => {
            return Err(ctx(format!(
                "unknown on_violation '{other}' -- one of log, console, stop, pause"
            )))
        }
    };
    let hold = match field("hold") {
        None => None,
        Some(raw) => {
            Some(parse_number(raw).ok_or_else(|| ctx(format!("'hold' is not a number: '{raw}'")))?)
        }
    };

    let qualifiers = |min_duration: Option<f64>| Qualifiers {
        min_duration,
        hysteresis: None,
        after: None,
        before: None,
        max_events: None,
        stop_after: None,
    };

    let mut rules = Vec::new();
    for (key, raw, _) in &written {
        let limit = match key.as_str() {
            "on_violation" | "severity" | "hold" => continue,
            _ => {
                parse_number(raw).ok_or_else(|| ctx(format!("'{key}' is not a number: '{raw}'")))?
            }
        };
        if limit <= 0.0 {
            // A magnitude limit of zero or less is violated at rest, so every step of every
            // run would report. It is a typo, not a rule.
            return Err(ctx(format!(
                "'{key}' must be greater than zero, not '{raw}' -- it is a limit on magnitude"
            )));
        }

        let (quantity, unit) = match key.as_str() {
            "i_max" | "i_sat" => (branch_current(element).map_err(ctx)?, " A"),
            "v_max" => (element_voltage(element).map_err(ctx)?, " V"),
            _ => unreachable!("key came from RATING_FIELDS"),
        };

        rules.push(Rule {
            // `target` is the label every report prints before the rule's own name, and the
            // thing a schematic view would highlight, so it is the component rather than the
            // quantity: `L1.i_max`, not `I(L1).I(L1)` or the `V(out,0).v_max` a voltage
            // rating's quantity would produce, which names a node pair and not the capacitor
            // that is over its rating. The quantity stays in the `Watch`, where it is what
            // gets read.
            name: key.clone(),
            target: name.clone(),
            watch: Watch::Quantity { quantity },
            unit: unit.to_string(),
            condition: Condition::Outside {
                low: -limit,
                high: limit,
            },
            qualifiers: qualifiers(hold),
            severity,
            action,
        });
    }

    Ok((rules, cuts))
}

/// The waveform name of `element`'s own current, or why it does not have one.
///
/// Only `V`, `L`, `E` and `H` carry a branch current as an MNA unknown. A resistor's,
/// capacitor's or ideal switch's current is a product of the solution rather than part of it,
/// so there is no `I(name)` column to watch -- as the CSV header of any switching deck shows.
///
/// This is an error rather than a warning because of what the alternative does: a rating that
/// is accepted and never evaluated reads, in the summary table, exactly like a rating that was
/// never exceeded.
fn branch_current(element: &ElementInstance) -> Result<String, String> {
    match element.device_letter {
        'V' | 'L' | 'E' | 'H' => Ok(format!("I({})", element.name)),
        letter => Err(format!(
            "a current rating needs a branch current, and device {letter} does not have one \
             (only V, L, E and H do -- a resistor's or a switch's current is computed from the \
             solution, not solved for). Write a `kind=rule` card naming a current you do have, \
             or probe this one first."
        )),
    }
}

/// The waveform expression for the voltage across a two-terminal `element`.
///
/// `V(p,n)` rather than `V(p)`: a rating is across the component, and the component is rarely
/// referenced to ground. [`StepSample::get`] evaluates the difference.
fn element_voltage(element: &ElementInstance) -> Result<String, String> {
    match element.nodes.as_slice() {
        [positive, negative] => Ok(format!("V({positive},{negative})")),
        nodes => Err(format!(
            "a voltage rating needs two terminals, and this device has {} \
             (write a `kind=rule` card naming the voltage you mean)",
            nodes.len()
        )),
    }
}

/// The `period=`/`sync=` pair every per-period measurement shares.
///
/// Exactly one of the two. Both would be a contradiction and neither leaves no period to
/// measure over, and in both cases the deck's author meant something specific.
fn parse_boundary(
    fields: &BTreeMap<&str, &str>,
    ctx: &impl Fn(String) -> String,
) -> Result<PeriodBoundary, String> {
    let period = match fields.get("period").copied() {
        None => None,
        Some(raw) => Some(
            parse_number(raw)
                .ok_or_else(|| ctx(format!("field 'period' is not a number: '{raw}'")))?,
        ),
    };
    match (period, fields.get("sync").copied()) {
        (Some(_), Some(_)) => Err(ctx(
            "'period' and 'sync' both given -- a period is either stated or taken from a \
             switch, not both"
                .into(),
        )),
        (None, None) => Err(ctx(
            "needs 'period=<seconds>' or 'sync=<switch>' to know what one period is".into(),
        )),
        (Some(period), None) => {
            if period <= 0.0 {
                return Err(ctx(format!(
                    "'period' must be greater than zero, not '{period}'"
                )));
            }
            Ok(PeriodBoundary::Fixed(period))
        }
        (None, Some(switch)) => Ok(PeriodBoundary::Sync(switch.to_string())),
    }
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
        "switch_timing" => {
            let switches = list("switches")?;
            let target = fields
                .get("name")
                .copied()
                .map(str::to_string)
                .unwrap_or_else(|| switches.join("+"));
            // Defaults to on, and not only because it is the cheapest of the three checks: a
            // switch that never commutated cannot have produced a measurable on-time or
            // frequency either, so a rule that asked for those and stayed quiet would be
            // reporting a pass it never established.
            let require_commutation = match fields.get("require_commutation").copied() {
                None | Some("yes") | Some("true") | Some("1") => true,
                Some("no") | Some("false") | Some("0") => false,
                Some(other) => {
                    return Err(ctx(format!(
                        "require_commutation must be yes or no, not '{other}'"
                    )))
                }
            };
            let watch = Watch::SwitchTiming {
                switches,
                min_on: number_opt("min_on")?,
                min_off: number_opt("min_off")?,
                f_min: number_opt("f_min")?,
                f_max: number_opt("f_max")?,
                require_commutation,
            };
            (
                watch,
                // Every finding a timing rule makes is an instant carrying its own value,
                // limit and unit, so the interval machinery this condition drives is never
                // reached. It is here because `Rule` requires one.
                Condition::Above { limit: 0.0 },
                target,
                String::new(),
            )
        }
        "rate_of_change" => {
            let quantity = fields
                .get("target")
                .copied()
                .ok_or_else(|| ctx("missing field 'target'".into()))?
                .to_string();
            let max_slope = number("max_slope")?;
            if max_slope <= 0.0 {
                return Err(ctx(format!(
                    "'max_slope' must be greater than zero, not '{max_slope}' -- it bounds a \
                     magnitude"
                )));
            }
            (
                Watch::RateOfChange {
                    quantity: quantity.clone(),
                    max_slope,
                },
                Condition::Above { limit: max_slope },
                quantity,
                fields
                    .get("unit")
                    .copied()
                    .map(str::to_string)
                    .unwrap_or_else(|| " /s".to_string()),
            )
        }
        "body_diode" => {
            let switches = list("switches")?;
            let currents = list("currents")?;
            if currents.len() != switches.len() {
                return Err(ctx(format!(
                    "'currents' lists {} name(s) for {} switch(es) -- a switch's own current \
                     is not an MNA unknown, so each one has to be named",
                    currents.len(),
                    switches.len()
                )));
            }
            let i_min = number("i_min")?;
            let target = fields
                .get("name")
                .copied()
                .map(str::to_string)
                .unwrap_or_else(|| switches.join("+"));
            (
                Watch::BodyDiode {
                    switches,
                    i_min,
                    currents,
                },
                Condition::Above { limit: i_min },
                target,
                " A".to_string(),
            )
        }
        "per_period" => {
            let quantity = fields
                .get("target")
                .copied()
                .ok_or_else(|| ctx("missing field 'target'".into()))?
                .to_string();
            let measure = match fields.get("measure").copied() {
                Some("rms") => PeriodMeasure::Rms,
                Some("mean") => PeriodMeasure::Mean,
                Some("abs_mean") => PeriodMeasure::AbsMean,
                Some("integral") => PeriodMeasure::Integral,
                None => return Err(ctx("missing field 'measure'".into())),
                Some(other) => {
                    return Err(ctx(format!(
                        "unknown measure '{other}' -- one of rms, mean, abs_mean, integral"
                    )))
                }
            };
            let boundary = parse_boundary(fields, &ctx)?;
            let (min, max) = (number_opt("min")?, number_opt("max")?);
            if min.is_none() && max.is_none() {
                return Err(ctx(
                    "needs 'min' or 'max' -- a measure with no bound is a measurement, and \
                     `kind=measure` already makes those"
                        .into(),
                ));
            }
            (
                Watch::PerPeriod {
                    quantity: quantity.clone(),
                    measure,
                    boundary,
                    min,
                    max,
                },
                Condition::Above {
                    limit: max.or(min).unwrap_or(0.0),
                },
                quantity,
                fields.get("unit").copied().unwrap_or("").to_string(),
            )
        }
        "efficiency" => {
            let port = |key: &str| -> Result<String, String> {
                Ok(fields
                    .get(key)
                    .copied()
                    .ok_or_else(|| {
                        ctx(format!(
                            "missing field '{key}' -- the name of a `kind=port` declaration"
                        ))
                    })?
                    .to_string())
            };
            let numerator = port("out")?;
            let denominator = port("in")?;
            if numerator == denominator {
                return Err(ctx(format!(
                    "'in' and 'out' are both '{numerator}' -- a port's efficiency against \
                     itself is 1 by construction"
                )));
            }
            let boundary = parse_boundary(fields, &ctx)?;
            let (min, max) = (number_opt("min")?, number_opt("max")?);
            if min.is_none() && max.is_none() {
                return Err(ctx(
                    "needs 'min' or 'max' -- 'max=1' is the one that says this is a converter \
                     rather than a source of energy"
                        .into(),
                ));
            }
            (
                Watch::Efficiency {
                    numerator: numerator.clone(),
                    denominator: denominator.clone(),
                    boundary,
                    min,
                    max,
                },
                Condition::Above {
                    limit: max.or(min).unwrap_or(0.0),
                },
                format!("{numerator}/{denominator}"),
                // Dimensionless by construction, which is the whole reason the rule is a ratio
                // and not a bounded difference of two powers.
                fields.get("unit").copied().unwrap_or("").to_string(),
            )
        }
        "commutation_order" => {
            let sequence = list("sequence")?;
            if sequence.len() < 2 {
                return Err(ctx(
                    "'sequence' needs at least two switches -- an order of one is not an order"
                        .into(),
                ));
            }
            let target = fields
                .get("name")
                .copied()
                .map(str::to_string)
                .unwrap_or_else(|| sequence.join("->"));
            (
                Watch::CommutationOrder { sequence },
                Condition::Above { limit: 0.0 },
                target,
                String::new(),
            )
        }
        other => {
            return Err(ctx(format!(
                "unknown rule type '{other}' -- one of above, below, outside, inside, \
                 forbidden_state, required_path, switch_timing, rate_of_change, body_diode, \
                 commutation_order, per_period, efficiency"
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
    /// Declared [`Port`]s, whose names resolve to a power rather than to an unknown.
    pub ports: &'a BTreeMap<String, Port>,
    /// Every switch's channel, so `P_cond(NAME)` resolves to its conduction power.
    pub switches: &'a BTreeMap<String, SwitchChannel>,
}

impl Sample for StepSample<'_> {
    fn get(&self, quantity: &str) -> Option<f64> {
        // A declared port resolves first, and to a power. It is checked before the solver's own
        // names because a port is a name the deck's author introduced deliberately: if it
        // collided with an unknown, silently preferring the unknown would answer a different
        // question from the one the rule asks.
        if let Some(port) = self.ports.get(quantity) {
            return self.port_power(port);
        }
        // `P_cond(NAME)`: a switch's channel conduction power. Resolved as a quantity rather
        // than built as its own `Watch` variant, so that every existing rule type, qualifier
        // and per-period measure applies to a loss without the engine learning anything about
        // switches it does not already know.
        if let Some(switch) = quantity
            .strip_prefix("P_cond(")
            .and_then(|rest| rest.strip_suffix(')'))
        {
            return self.conduction_power(switch.trim());
        }
        // The same names the waveform's own header carries, so a rule is written against what
        // the user already sees in the CSV rather than a second naming scheme.
        if let Some(value) = self.point.value(quantity) {
            return Some(value);
        }
        if let Some(SignalValue::Scalar(v)) = self.outputs.get(quantity) {
            return Some(*v);
        }
        // `V(p,n)`: the voltage across something, which is what a rating on a two-terminal
        // device means and what anyone writing a rule about a floating component wants. Not an
        // unknown, so it is computed -- and `0` is ground, which has no unknown of its own.
        if let Some(pair) = quantity
            .strip_prefix("V(")
            .or_else(|| quantity.strip_prefix("v("))
            .and_then(|rest| rest.strip_suffix(')'))
        {
            if let Some((positive, negative)) = pair.split_once(',') {
                let node = |name: &str| -> Option<f64> {
                    let name = name.trim();
                    if name == "0" {
                        return Some(0.0);
                    }
                    self.point.value(&format!("V({name})"))
                };
                return Some(node(positive)? - node(negative)?);
            }
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

    fn body_diode_on(&self, switch: &str) -> Option<bool> {
        // A switch appears in the step's segment list only while it is commanded off: an on
        // switch is stamped as a plain `r_on` resistance, with no segment to resolve. So
        // `None` here means "commanded on, or not a switch", and `Some(false)` means
        // "commanded off and blocking" -- the distinction the rule depends on.
        let position = self.point.diode_names.iter().position(|n| n == switch)?;
        let (breakdown, forward) = self.point.diode_z.get(position)?;
        // `z1 > 0` is the breakdown segment and `z2 > 0` the forward one; both at zero is the
        // blocking segment between them. A body diode conducting in either direction carries
        // current, which is what the rule asks about.
        Some(*breakdown > 0.0 || *forward > 0.0)
    }
}

impl StepSample<'_> {
    /// A port's instantaneous power, $p = v \cdot i$, with `i` negated only if the deck asked.
    ///
    /// No hidden convention: see [`Port`] for why a built-in negation cannot serve both ends of
    /// a converter, and for the two canonical declarations.
    /// A switch's channel conduction power, $p = (V_a - V_b)^2 / r_{on}$ while commanded on and
    /// exactly zero while commanded off.
    ///
    /// ## Why the gating is the whole correctness argument
    ///
    /// A switch commanded on is stamped as a plain `r_on` resistance between its two declared
    /// nodes, so its current is not an unknown but is exactly computable. Measured against a
    /// zero-volt sense source in series with the switch of a buck, the worst relative
    /// disagreement over a run was $2.7 \times 10^{-9}$.
    ///
    /// While commanded **off** the same expression is not merely imprecise, it is nonsense: the
    /// same run gave 1240 A against a true 0 A, because the device is no longer a resistance and
    /// the voltage across it is the full blocking voltage. So the state has to gate the
    /// expression, and a test asserts that what happens during the off time cannot change the
    /// reported loss.
    ///
    /// Zero rather than unreadable while off, because the channel genuinely dissipates nothing
    /// then -- which also makes a per-period mean come out as the duty-weighted average without
    /// the rule having to know the duty. Body-diode conduction is a different expression on a
    /// different state and is deliberately not counted here.
    fn conduction_power(&self, switch: &str) -> Option<f64> {
        let channel = self.switches.get(switch)?;
        if !self.switch_on(switch)? {
            return Some(0.0);
        }
        let node = |name: &str| -> Option<f64> {
            if name == "0" {
                return Some(0.0);
            }
            self.point.value(&format!("V({name})"))
        };
        let across = node(&channel.positive)? - node(&channel.negative)?;
        Some(across * across / channel.r_on)
    }

    fn port_power(&self, port: &Port) -> Option<f64> {
        let node = |name: &str| -> Option<f64> {
            if name == "0" {
                return Some(0.0);
            }
            self.point.value(&format!("V({name})"))
        };
        let voltage = node(&port.positive)? - node(&port.negative)?;
        let current = self.point.value(&format!("I({})", port.branch))?;
        Some(voltage * if port.negate { -current } else { current })
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

    /// Compiles `source`'s ratings, returning the stripped netlist and the rules.
    fn ratings(source: &str) -> (String, Vec<Rule>) {
        let out = extract(source, Dialect::Ngspice).expect("the deck parses");
        (out.netlist, out.rules)
    }

    /// Compiles `source`, returning its declared ports.
    fn declared_ports(source: &str) -> BTreeMap<String, Port> {
        extract(source, Dialect::Ngspice)
            .expect("the deck parses")
            .ports
    }

    fn rating_error(source: &str) -> String {
        extract(source, Dialect::Ngspice).expect_err("the deck should be rejected")
    }

    #[test]
    fn a_rating_compiles_to_a_magnitude_rule_on_the_elements_own_current() {
        let (_, rules) = ratings("V1 a 0 10\nL1 a 0 36u i_max=60\n");
        assert_eq!(rules.len(), 1);
        let rule = &rules[0];
        assert_eq!(rule.name, "i_max");
        assert_eq!(rule.target, "L1");
        assert_eq!(rule.unit, " A");
        assert_eq!(
            rule.watch,
            Watch::Quantity {
                quantity: "I(L1)".to_string()
            }
        );
        // The claim worth testing: a rating is on magnitude. `above(60)` would let a 70 A
        // reverse current through a bridge inductor pass unreported.
        assert_eq!(
            rule.condition,
            Condition::Outside {
                low: -60.0,
                high: 60.0
            }
        );
        assert!(rule.condition.margin(-70.0) > 0.0, "a reverse overload");
        assert!(rule.condition.margin(70.0) > 0.0, "a forward overload");
        assert!(rule.condition.margin(59.0) <= 0.0, "within the rating");
    }

    #[test]
    fn a_rating_defaults_to_error_where_a_card_defaults_to_warning() {
        // A datasheet number is not a matter of taste, so the default is one step up from a
        // card's, and the action follows from the severity exactly as a card's does.
        let (_, rules) = ratings("V1 a 0 10\nL1 a 0 36u i_max=60\n");
        assert_eq!(rules[0].severity, Severity::Error);
        assert_eq!(rules[0].action, Action::Console);
    }

    #[test]
    fn a_voltage_rating_watches_the_difference_across_the_component() {
        let (_, rules) = ratings("V1 a 0 10\nC1 a b 1u v_max=400\n");
        assert_eq!(
            rules[0].watch,
            Watch::Quantity {
                quantity: "V(a,b)".to_string()
            }
        );
        assert_eq!(rules[0].unit, " V");
        assert_eq!(rules[0].target, "C1");
    }

    #[test]
    fn modifiers_apply_to_every_rating_on_the_line() {
        let (_, rules) = ratings(
            "V1 a 0 10\nL1 a 0 36u i_max=60 i_sat=70 on_violation=stop severity=warning hold=2u\n",
        );
        assert_eq!(rules.len(), 2, "i_max and i_sat, not the three modifiers");
        for rule in &rules {
            assert_eq!(rule.action, Action::Stop);
            assert_eq!(rule.severity, Severity::Warning);
            assert_eq!(rule.qualifiers.min_duration, Some(2e-6));
        }
        let names: Vec<&str> = rules.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["i_max", "i_sat"]);
    }

    #[test]
    fn the_rating_leaves_the_device_line_otherwise_untouched() {
        // The stripping is what keeps a rating out of the deck hash, so an edited limit still
        // loads its checkpoint. The rest of the line has to survive it.
        let (stripped, _) = ratings("V1 a 0 10\nL1 a 0 36u i_max=60 ic=2\n");
        assert_eq!(stripped, "V1 a 0 10\nL1 a 0 36u ic=2\n");
    }

    #[test]
    fn two_decks_differing_only_in_a_limit_strip_to_the_same_text() {
        // Stated directly, because this is the property the whole stripping exists for: the
        // deck hash a checkpoint is keyed by cannot see a rating.
        let (low, _) = ratings("V1 a 0 10\nL1 a 0 36u i_max=60\n");
        let (high, _) = ratings("V1 a 0 10\nL1 a 0 36u i_max=4000\n");
        assert_eq!(low, high);
    }

    #[test]
    fn a_rating_on_a_continuation_line_is_cut_from_that_line() {
        let (stripped, rules) = ratings("V1 a 0 10\nL1 a 0 36u\n+ i_max=60 ic=2\n");
        assert_eq!(rules.len(), 1);
        assert_eq!(stripped, "V1 a 0 10\nL1 a 0 36u\n+ ic=2\n");
    }

    #[test]
    fn a_current_rating_needs_a_device_that_has_a_current() {
        // Accepting it would be worse than refusing it: in the summary table a rating that was
        // never evaluated reads exactly like one that was never exceeded.
        let error = rating_error("V1 a 0 10\nR1 a 0 50 i_max=2\n");
        assert!(error.contains("device R does not have one"), "{error}");
        assert!(error.contains("only V, L, E and H"), "{error}");
        assert!(error.starts_with("line 2: device 'R1'"), "{error}");
    }

    #[test]
    fn a_modifier_without_a_rating_is_refused() {
        // `on_violation=stop` alone reads like a guarded deck and guards nothing.
        let error = rating_error("V1 a 0 10\nL1 a 0 36u on_violation=stop\n");
        assert!(error.contains("modifies a rating"), "{error}");
    }

    #[test]
    fn a_limit_of_zero_or_less_is_a_typo_not_a_rule() {
        for raw in ["0", "-5"] {
            let error = rating_error(&format!("V1 a 0 10\nL1 a 0 36u i_max={raw}\n"));
            assert!(
                error.contains("must be greater than zero"),
                "{raw}: {error}"
            );
        }
    }

    #[test]
    fn a_rating_that_is_not_a_number_says_so() {
        let error = rating_error("V1 a 0 10\nL1 a 0 36u i_max=plenty\n");
        assert!(error.contains("'i_max' is not a number"), "{error}");
    }

    #[test]
    fn general_mna_accepts_every_rating_field_once_it_is_registered() {
        // This crate strips ratings before anything builds the deck, so its own run never needs
        // the registration. Another consumer's does: a GUI or a tool that hands the netlist
        // straight to `general-mna` sees a hard "unknown field" error unless these names are
        // registered. The contract between the two halves is asserted here rather than left to
        // a comment, because drift would be silent until someone wrote the field in a deck.
        let mut options = general_mna::BuildOptions::default();
        for field in RATING_FIELDS {
            options.annotate(field);
        }
        let deck = "V1 a 0 10\nL1 a 0 36u i_max=60 i_sat=70 v_max=400 \
                    on_violation=console severity=error hold=2u\n";
        let system = general_mna::MnaBuilder::with_options(Dialect::Ngspice, options)
            .build_fragment(deck)
            .expect("general-mna must accept every field this crate claims");

        let mut carried: Vec<&str> = system.annotations.iter().map(|a| a.key.as_str()).collect();
        carried.sort_unstable();
        let mut expected: Vec<&str> = RATING_FIELDS.to_vec();
        expected.sort_unstable();
        assert_eq!(
            carried, expected,
            "every claimed field must survive the build"
        );
    }

    #[test]
    fn a_deck_with_no_ratings_comes_back_unchanged() {
        let source = "V1 a 0 10\nR1 a 0 50\n";
        let (stripped, rules) = ratings(source);
        assert_eq!(stripped, source);
        assert!(rules.is_empty());
    }

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

    // ---- declared ports --------------------------------------------------

    #[test]
    fn a_port_declaration_is_taken_out_of_the_netlist_and_parsed() {
        let deck = "V1 in 0 10\nR1 in 0 5\nP_IN kind=port v=(in,0) i=-V1\n";
        let ports = declared_ports(deck);
        assert_eq!(ports.len(), 1);
        let port = &ports["P_IN"];
        assert_eq!(port.positive, "in");
        assert_eq!(port.negative, "0");
        assert_eq!(port.branch, "V1");
        assert!(
            port.negate,
            "the leading minus is the whole sign convention"
        );

        // The solver would reject `kind=port` as an unknown device, so the line has to be gone
        // -- and gone as a blank line, so every other statement keeps its own line number.
        let (netlist, _) = ratings(deck);
        assert!(!netlist.contains("kind=port"), "{netlist}");
        assert_eq!(netlist.lines().count(), deck.lines().count(), "{netlist}");
    }

    #[test]
    fn a_port_on_a_sense_element_carries_no_minus() {
        let ports = declared_ports(
            "V1 in 0 10\nVSENSE out load 0\nR1 load 0 5\nP_OUT kind=port v=(out,0) i=VSENSE\n",
        );
        assert!(
            !ports["P_OUT"].negate,
            "a series sense element's current already flows into its positive terminal"
        );
    }

    #[test]
    fn a_port_missing_either_half_of_its_definition_is_refused() {
        // A port is a voltage *and* a current. Either alone is not a power, and defaulting the
        // missing half would mean guessing which boundary the author meant.
        let no_current = rating_error("V1 in 0 10\nP kind=port v=(in,0)\n");
        assert!(
            no_current.contains("missing field 'i'") && no_current.contains("sense source"),
            "{no_current}"
        );
        let no_voltage = rating_error("V1 in 0 10\nP kind=port i=V1\n");
        assert!(
            no_voltage.contains("missing field 'v'") && no_voltage.contains("v=(p,n)"),
            "{no_voltage}"
        );
    }

    #[test]
    fn a_port_whose_node_pair_is_malformed_names_the_shape_it_wanted() {
        for bad in ["v=in", "v=(in)", "v=(,0)", "v=(in,)"] {
            let message = rating_error(&format!("V1 in 0 10\nP kind=port {bad} i=V1\n"));
            assert!(
                message.contains("field 'v'"),
                "'{bad}' should be refused by name: {message}"
            );
        }
    }

    #[test]
    fn two_ports_of_one_name_are_refused() {
        // A rule names a port by name. Two of them would make `in=P` mean either, and silently
        // keeping the last would make the deck's meaning depend on line order.
        let message = rating_error(
            "V1 in 0 10\nR1 in 0 5\nP kind=port v=(in,0) i=V1\nP kind=port v=(in,0) i=-V1\n",
        );
        assert!(message.contains("declared twice"), "{message}");
    }

    // ---- the efficiency card ---------------------------------------------

    #[test]
    fn an_efficiency_card_compiles_to_a_ratio_of_two_ports() {
        let (_, rules) = ratings(
            "V1 in 0 10\nR1 in 0 5\n\
             P_IN kind=port v=(in,0) i=-V1\n\
             P_OUT kind=port v=(in,0) i=V1\n\
             ETA kind=rule type=efficiency in=P_IN out=P_OUT max=1 period=1e-5\n",
        );
        assert_eq!(rules.len(), 1);
        let rule = &rules[0];
        // The target is what the report prints, and it has to read as the ratio it is.
        assert_eq!(rule.target, "P_OUT/P_IN");
        assert_eq!(
            rule.watch,
            Watch::Efficiency {
                numerator: "P_OUT".to_string(),
                denominator: "P_IN".to_string(),
                boundary: PeriodBoundary::Fixed(1e-5),
                min: None,
                max: Some(1.0),
            }
        );
    }

    #[test]
    fn an_efficiency_card_with_no_bound_is_refused_and_recommends_max_one() {
        let message = rating_error(
            "V1 in 0 10\nP_IN kind=port v=(in,0) i=-V1\nP_OUT kind=port v=(in,0) i=V1\n\
             ETA kind=rule type=efficiency in=P_IN out=P_OUT period=1e-5\n",
        );
        assert!(
            message.contains("needs 'min' or 'max'") && message.contains("max=1"),
            "{message}"
        );
    }

    #[test]
    fn an_efficiency_of_a_port_against_itself_is_refused() {
        let message = rating_error(
            "V1 in 0 10\nP kind=port v=(in,0) i=-V1\n\
             ETA kind=rule type=efficiency in=P out=P max=1 period=1e-5\n",
        );
        assert!(message.contains("against itself"), "{message}");
    }

    #[test]
    fn an_efficiency_card_needs_exactly_one_period_boundary() {
        let both = rating_error(
            "V1 in 0 10\nP_IN kind=port v=(in,0) i=-V1\nP_OUT kind=port v=(in,0) i=V1\n\
             ETA kind=rule type=efficiency in=P_IN out=P_OUT max=1 period=1e-5 sync=DSW\n",
        );
        assert!(both.contains("both given"), "{both}");
        let neither = rating_error(
            "V1 in 0 10\nP_IN kind=port v=(in,0) i=-V1\nP_OUT kind=port v=(in,0) i=V1\n\
             ETA kind=rule type=efficiency in=P_IN out=P_OUT max=1\n",
        );
        assert!(neither.contains("what one period is"), "{neither}");
    }

    // ---- a switch's channel, and its conduction power --------------------

    /// The channels `source` declares.
    fn channels(source: &str) -> BTreeMap<String, SwitchChannel> {
        extract(source, Dialect::Ngspice)
            .expect("the deck parses")
            .switches
    }

    const ONE_SWITCH: &str = "\
VIN in 0 12
DSW in mid idealswitchmodel
DSW kind=ideal_switch r_on=0.02 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.5 g_on=100 gate=block ctrl=G
R1 mid 0 5
D kind=const value=1
G kind=sig2phys domain=voltage in=D
";

    #[test]
    fn a_switch_channel_is_assembled_from_the_two_lines_that_share_its_name() {
        // The device line carries the nodes, the `kind=ideal_switch` line carries `r_on`, and
        // neither is restated on a rule card.
        let channels = channels(ONE_SWITCH);
        assert_eq!(channels.len(), 1);
        let channel = &channels["DSW"];
        assert_eq!(channel.positive, "in");
        assert_eq!(channel.negative, "mid");
        assert_eq!(channel.r_on, 0.02);
    }

    #[test]
    fn the_ideal_switch_line_stays_in_the_netlist_after_extraction() {
        // Read, not stripped -- unlike a rule card or a port. The solver needs it.
        let (netlist, _) = ratings(ONE_SWITCH);
        assert!(
            netlist.contains("kind=ideal_switch"),
            "the solver still needs this line:\n{netlist}"
        );
    }

    /// A `StepSample` over hand-written node voltages, for asserting the formula itself.
    fn sample_with<'a>(
        unknowns: &[&str],
        x: &[f64],
        gates: &'a BTreeMap<String, GateState>,
        switches: &'a BTreeMap<String, SwitchChannel>,
        point: &'a mut OperatingPoint,
        outputs: &'a BTreeMap<String, SignalValue>,
        ports: &'a BTreeMap<String, Port>,
    ) -> StepSample<'a> {
        point.unknowns = unknowns.iter().map(|s| s.to_string()).collect();
        point.x = x.to_vec();
        StepSample {
            point,
            outputs,
            gates,
            ports,
            switches,
        }
    }

    #[test]
    fn a_commanded_on_switch_dissipates_the_square_of_its_drop_over_r_on() {
        let switches = BTreeMap::from([(
            "DSW".to_string(),
            SwitchChannel {
                positive: "in".to_string(),
                negative: "mid".to_string(),
                r_on: 0.02,
            },
        )]);
        let gates = BTreeMap::from([("DSW".to_string(), GateState::On)]);
        let (outputs, ports) = (BTreeMap::new(), BTreeMap::new());
        let mut point = OperatingPoint {
            unknowns: Vec::new(),
            x: Vec::new(),
            diode_names: Vec::new(),
            diode_z: Vec::new(),
            diode_raw_ioff: Vec::new(),
        };
        // A 0.02 V drop across 0.02 ohm is 1 A, so 0.02 W.
        let sample = sample_with(
            &["V(in)", "V(mid)"],
            &[12.0, 11.98],
            &gates,
            &switches,
            &mut point,
            &outputs,
            &ports,
        );
        let power = sample.get("P_cond(DSW)").expect("a commanded-on switch");
        assert!(
            (power - 0.02).abs() < 1e-12,
            "(0.02 V)^2 / 0.02 ohm = 0.02 W, got {power}"
        );
    }

    #[test]
    fn a_commanded_off_switch_dissipates_exactly_zero_whatever_is_across_it() {
        // The expression is not merely imprecise while off, it is nonsense: on a real run the
        // same arithmetic gave 1240 A against a true 0 A, because the device is no longer a
        // resistance and the voltage across it is the full blocking voltage. The gating on the
        // commanded state is the whole correctness argument, so it is asserted directly.
        let switches = BTreeMap::from([(
            "DSW".to_string(),
            SwitchChannel {
                positive: "in".to_string(),
                negative: "mid".to_string(),
                r_on: 0.02,
            },
        )]);
        let gates = BTreeMap::from([("DSW".to_string(), GateState::Off)]);
        let (outputs, ports) = (BTreeMap::new(), BTreeMap::new());
        let mut point = OperatingPoint {
            unknowns: Vec::new(),
            x: Vec::new(),
            diode_names: Vec::new(),
            diode_z: Vec::new(),
            diode_raw_ioff: Vec::new(),
        };
        // The full bus across it: 12^2 / 0.02 would be 7200 W if the state were ignored.
        let sample = sample_with(
            &["V(in)", "V(mid)"],
            &[12.0, 0.0],
            &gates,
            &switches,
            &mut point,
            &outputs,
            &ports,
        );
        assert_eq!(sample.get("P_cond(DSW)"), Some(0.0));
    }

    #[test]
    fn an_undeclared_switch_has_no_conduction_power_rather_than_zero() {
        // Unreadable, not zero: a rule naming a switch the netlist does not have must report as
        // unanswerable, exactly as every other missing quantity does. Zero would make a typo in
        // a switch name look like a lossless device.
        let (switches, gates, outputs, ports) = (
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
        );
        let mut point = OperatingPoint {
            unknowns: Vec::new(),
            x: Vec::new(),
            diode_names: Vec::new(),
            diode_z: Vec::new(),
            diode_raw_ioff: Vec::new(),
        };
        let sample = sample_with(
            &["V(in)"],
            &[12.0],
            &gates,
            &switches,
            &mut point,
            &outputs,
            &ports,
        );
        assert_eq!(sample.get("P_cond(NOPE)"), None);
    }
}
