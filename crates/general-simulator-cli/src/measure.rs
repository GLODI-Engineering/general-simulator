//! `kind=measure` netlist statements: ngspice `.measure`/Xyce `.MEASURE`-style post-processing
//! measurements over a **completed** transient trace.
//!
//! # Why this lives here, not in `general-mna`/`dae-runtime`
//!
//! A measurement (`MAX`, `RMS`, `TRIG`/`TARG`, `FOUR`, ...) is evaluated exactly once, after a
//! simulation finishes, over the full resolved waveform — it has no bearing on circuit topology
//! or per-step block evaluation, so it does not belong in `general-mna`'s `BlockKind`/block
//! graph (`dae-runtime` never sees it). See `book/dev-guide/src/measurements.md` for the full
//! architecture rationale. This module recognizes and strips `kind=measure` lines from the
//! netlist text *before* it reaches `general_mna::build_system` (see [`extract`]), so
//! `general-mna` never has to know `measure` is a `kind=` value at all — no changes to
//! `general-mna` or `spice-lsp` were needed for this feature.
//!
//! The actual measurement math (Δt-weighted scalar reductions, straddling-sample threshold
//! crossings, exact per-segment analytic Fourier analysis, error norms) is not reimplemented
//! here — every measurement in this module is a thin field-parsing/dispatch layer over the
//! `gs-waveform-measurements` crate; see that crate's own `src/lib.rs` doc comment for the
//! numerical techniques and why they matter for a variable-timestep trace.

use std::collections::BTreeMap;

use general_spice_core::ast::{BlockInstance as AstBlockInstance, Statement};
use general_spice_core::Dialect;
use gs_waveform_measurements::{
    avg, deriv_at, deriv_when, err1, err2, error_norm, find_at, find_when, four, freq, integ, max,
    min, off_time, on_time, peak_to_peak, resolve_event, rms, when, Edge, EventSpec, Norm,
    Occurrence, Sample, Threshold, Window,
};

/// One parsed `kind=measure` line: a name (its own block name in the netlist) plus what to
/// compute. Resolved lazily against the completed trace by [`evaluate_all`].
pub struct MeasureSpec {
    pub name: String,
    pub line: usize,
    pub kind: MeasureKind,
}

/// Which side of a `WHEN`/`TRIG`/`TARG` threshold clause is given: a fixed level (`*_val=`) or
/// another signal (`*_ref=`) — ngspice/Xyce's `WHEN <var>=<value>` vs. `WHEN <var>=<variable2>`.
pub enum ThresholdCfg {
    Value(f64),
    Ref(String),
}

/// Which occurrence of which edge direction to accept — ngspice/Xyce's `RISE=`/`FALL=`/`CROSS=`
/// qualifiers, `n` (1-based) or `LAST`.
pub struct CrossingCfg {
    pub edge: Edge,
    pub occurrence: Occurrence,
}

/// The trigger/target event specification shared by `TRIG`/`TARG` clauses in a `trig_targ`
/// measurement — an owned counterpart to `gs_waveform_measurements::EventSpec` that names its
/// signal(s) by string instead of borrowing `Sample` slices, since those aren't available until
/// the trace is resolved.
pub enum EventCfg {
    At(f64),
    Crossing {
        var: String,
        threshold: ThresholdCfg,
        window: Window,
        crossing: CrossingCfg,
    },
    FracMax {
        var: String,
        frac: f64,
        window: Window,
        crossing: CrossingCfg,
    },
}

/// The specific measurement to compute — one variant per `type=` value, see
/// `book/user-guide/src/measurements.md` for the full field reference of each.
pub enum MeasureKind {
    Max {
        out: String,
        window: Window,
    },
    Min {
        out: String,
        window: Window,
    },
    MaxAt {
        out: String,
        window: Window,
    },
    MinAt {
        out: String,
        window: Window,
    },
    Pp {
        out: String,
        window: Window,
    },
    Avg {
        out: String,
        window: Window,
    },
    Rms {
        out: String,
        window: Window,
    },
    Integ {
        out: String,
        window: Window,
    },
    DerivAt {
        out: String,
        at: f64,
    },
    DerivWhen {
        out: String,
        when_var: String,
        threshold: ThresholdCfg,
        window: Window,
        crossing: CrossingCfg,
    },
    FindAt {
        out: String,
        at: f64,
    },
    FindWhen {
        out: String,
        when_var: String,
        threshold: ThresholdCfg,
        window: Window,
        crossing: CrossingCfg,
    },
    When {
        out: String,
        threshold: ThresholdCfg,
        window: Window,
        crossing: CrossingCfg,
    },
    TrigTarg {
        trig: EventCfg,
        targ: EventCfg,
    },
    Freq {
        out: String,
        on: f64,
        off: f64,
        window: Window,
    },
    OnTime {
        out: String,
        on: f64,
        off: f64,
        window: Window,
    },
    OffTime {
        out: String,
        on: f64,
        off: f64,
        window: Window,
    },
    Four {
        out: String,
        fundamental: f64,
        harmonics: usize,
        window: Window,
    },
    Err1 {
        out: String,
        reference: String,
        minval: f64,
        ymin: f64,
        ymax: f64,
    },
    Err2 {
        out: String,
        reference: String,
        minval: f64,
        ymin: f64,
        ymax: f64,
    },
    Error {
        out: String,
        reference: String,
        norm: Norm,
    },
}

/// Scans `source` for `kind=measure` `BlockInstance` lines, returning `(source with those exact
/// lines blanked out, the parsed measurement specs)`. Blanking (not deleting) preserves every
/// other statement's own line span, so re-parsing the returned text for `general_mna::
/// build_system` reports diagnostics at the same line numbers a user sees in their editor.
///
/// `general_mna::build_system` never sees `kind=measure` at all — its own `kind=` dispatch has
/// no entry for `"measure"` and would reject it as an unknown device kind (see
/// `general-mna/src/system_builder.rs`'s final `other => Err(...)` arm), which is exactly why
/// this filtering has to happen before that call, not after.
pub fn extract(source: &str, dialect: Dialect) -> Result<(String, Vec<MeasureSpec>), String> {
    let statements = general_mna::parse_and_flatten(source, dialect)?;
    let mut specs = Vec::new();
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
        if fields.get("kind").copied() != Some("measure") {
            continue;
        }
        specs.push(parse_measure_spec(bi, &fields)?);
        for line in bi.span.clone() {
            blank_lines.insert(line);
        }
    }

    if blank_lines.is_empty() {
        return Ok((source.to_string(), specs));
    }
    let mut out = String::with_capacity(source.len());
    for (i, line) in source.lines().enumerate() {
        let line_no = i + 1;
        if !blank_lines.contains(&line_no) {
            out.push_str(line);
        }
        out.push('\n');
    }
    Ok((out, specs))
}

fn parse_measure_spec(
    bi: &AstBlockInstance,
    fields: &BTreeMap<&str, &str>,
) -> Result<MeasureSpec, String> {
    let name = bi.name.clone();
    let line = bi.span.start;
    let ctx = |msg: String| -> String { format!("line {line}: measurement '{name}': {msg}") };

    let mtype = get_str(fields, "type").map_err(ctx)?;
    let out = get_str(fields, "out");

    let kind = match mtype.as_str() {
        "max" => MeasureKind::Max {
            out: out.map_err(ctx)?,
            window: window(fields, "").map_err(ctx)?,
        },
        "min" => MeasureKind::Min {
            out: out.map_err(ctx)?,
            window: window(fields, "").map_err(ctx)?,
        },
        "max_at" => MeasureKind::MaxAt {
            out: out.map_err(ctx)?,
            window: window(fields, "").map_err(ctx)?,
        },
        "min_at" => MeasureKind::MinAt {
            out: out.map_err(ctx)?,
            window: window(fields, "").map_err(ctx)?,
        },
        "pp" => MeasureKind::Pp {
            out: out.map_err(ctx)?,
            window: window(fields, "").map_err(ctx)?,
        },
        "avg" => MeasureKind::Avg {
            out: out.map_err(ctx)?,
            window: window(fields, "").map_err(ctx)?,
        },
        "rms" => MeasureKind::Rms {
            out: out.map_err(ctx)?,
            window: window(fields, "").map_err(ctx)?,
        },
        "integ" => MeasureKind::Integ {
            out: out.map_err(ctx)?,
            window: window(fields, "").map_err(ctx)?,
        },
        "deriv" => {
            let o = out.map_err(ctx)?;
            if let Some(at) = get_f64_opt(fields, "at").map_err(ctx)? {
                MeasureKind::DerivAt { out: o, at }
            } else {
                MeasureKind::DerivWhen {
                    out: o,
                    when_var: get_str(fields, "when").map_err(ctx)?,
                    threshold: threshold(fields, "when_").map_err(ctx)?,
                    window: window(fields, "").map_err(ctx)?,
                    crossing: crossing(fields, "").map_err(ctx)?,
                }
            }
        }
        "find" => {
            let o = out.map_err(ctx)?;
            if let Some(at) = get_f64_opt(fields, "at").map_err(ctx)? {
                MeasureKind::FindAt { out: o, at }
            } else {
                MeasureKind::FindWhen {
                    out: o,
                    when_var: get_str(fields, "when").map_err(ctx)?,
                    threshold: threshold(fields, "when_").map_err(ctx)?,
                    window: window(fields, "").map_err(ctx)?,
                    crossing: crossing(fields, "").map_err(ctx)?,
                }
            }
        }
        "when" => MeasureKind::When {
            out: out.map_err(ctx)?,
            threshold: threshold(fields, "when_").map_err(ctx)?,
            window: window(fields, "").map_err(ctx)?,
            crossing: crossing(fields, "").map_err(ctx)?,
        },
        "trig_targ" => MeasureKind::TrigTarg {
            trig: event_cfg(fields, "trig_").map_err(ctx)?,
            targ: event_cfg(fields, "targ_").map_err(ctx)?,
        },
        "freq" => MeasureKind::Freq {
            out: out.map_err(ctx)?,
            on: get_f64(fields, "on").map_err(ctx)?,
            off: get_f64(fields, "off").map_err(ctx)?,
            window: window(fields, "").map_err(ctx)?,
        },
        "on_time" => MeasureKind::OnTime {
            out: out.map_err(ctx)?,
            on: get_f64(fields, "on").map_err(ctx)?,
            off: get_f64(fields, "off").map_err(ctx)?,
            window: window(fields, "").map_err(ctx)?,
        },
        "off_time" => MeasureKind::OffTime {
            out: out.map_err(ctx)?,
            on: get_f64(fields, "on").map_err(ctx)?,
            off: get_f64(fields, "off").map_err(ctx)?,
            window: window(fields, "").map_err(ctx)?,
        },
        "four" => MeasureKind::Four {
            out: out.map_err(ctx)?,
            fundamental: get_f64(fields, "fundamental").map_err(ctx)?,
            harmonics: get_f64(fields, "harmonics").map_err(ctx)? as usize,
            window: window(fields, "").map_err(ctx)?,
        },
        "err1" => MeasureKind::Err1 {
            out: out.map_err(ctx)?,
            reference: get_str(fields, "ref").map_err(ctx)?,
            minval: get_f64_opt(fields, "minval").map_err(ctx)?.unwrap_or(1e-12),
            ymin: get_f64_opt(fields, "ymin").map_err(ctx)?.unwrap_or(0.0),
            ymax: get_f64_opt(fields, "ymax")
                .map_err(ctx)?
                .unwrap_or(f64::INFINITY),
        },
        "err2" => MeasureKind::Err2 {
            out: out.map_err(ctx)?,
            reference: get_str(fields, "ref").map_err(ctx)?,
            minval: get_f64_opt(fields, "minval").map_err(ctx)?.unwrap_or(1e-12),
            ymin: get_f64_opt(fields, "ymin").map_err(ctx)?.unwrap_or(0.0),
            ymax: get_f64_opt(fields, "ymax")
                .map_err(ctx)?
                .unwrap_or(f64::INFINITY),
        },
        "error" => {
            let norm = match fields.get("norm").copied() {
                Some("l1") => Norm::L1,
                Some("l2") | None => Norm::L2,
                Some("infnorm") => Norm::InfNorm,
                Some(other) => {
                    return Err(ctx(format!(
                        "unknown 'norm' value '{other}' (expected l1, l2, or infnorm)"
                    )))
                }
            };
            MeasureKind::Error {
                out: out.map_err(ctx)?,
                reference: get_str(fields, "ref").map_err(ctx)?,
                norm,
            }
        }
        other => {
            return Err(ctx(format!(
                "unknown measurement 'type' value '{other}' (expected one of: max, min, max_at, \
                 min_at, pp, avg, rms, integ, deriv, find, when, trig_targ, freq, on_time, \
                 off_time, four, err1, err2, error)"
            )))
        }
    };

    Ok(MeasureSpec { name, line, kind })
}

fn event_cfg(fields: &BTreeMap<&str, &str>, prefix: &str) -> Result<EventCfg, String> {
    if let Some(at) = get_f64_opt(fields, &format!("{prefix}at"))? {
        return Ok(EventCfg::At(at));
    }
    let var = get_str(fields, &format!("{prefix}var"))?;
    let win = window(fields, prefix)?;
    if let Some(frac) = get_f64_opt(fields, &format!("{prefix}frac_max"))? {
        return Ok(EventCfg::FracMax {
            var,
            frac,
            window: win,
            crossing: crossing(fields, prefix)?,
        });
    }
    Ok(EventCfg::Crossing {
        var,
        threshold: threshold(fields, prefix)?,
        window: win,
        crossing: crossing(fields, prefix)?,
    })
}

fn window(fields: &BTreeMap<&str, &str>, prefix: &str) -> Result<Window, String> {
    Ok(Window {
        from: get_f64_opt(fields, &format!("{prefix}from"))?,
        to: get_f64_opt(fields, &format!("{prefix}to"))?,
        td: get_f64_opt(fields, &format!("{prefix}td"))?,
    })
}

fn threshold(fields: &BTreeMap<&str, &str>, prefix: &str) -> Result<ThresholdCfg, String> {
    let val_key = format!("{prefix}val");
    let ref_key = format!("{prefix}ref");
    match (fields.get(val_key.as_str()), fields.get(ref_key.as_str())) {
        (Some(v), None) => Ok(ThresholdCfg::Value(
            v.parse::<f64>()
                .map_err(|_| format!("'{val_key}' is not a number"))?,
        )),
        (None, Some(r)) => Ok(ThresholdCfg::Ref(r.to_string())),
        (Some(_), Some(_)) => Err(format!(
            "'{val_key}' and '{ref_key}' are mutually exclusive"
        )),
        (None, None) => Err(format!("missing '{val_key}' or '{ref_key}'")),
    }
}

fn crossing(fields: &BTreeMap<&str, &str>, prefix: &str) -> Result<CrossingCfg, String> {
    let rise = fields.get(format!("{prefix}rise").as_str()).copied();
    let fall = fields.get(format!("{prefix}fall").as_str()).copied();
    let cross = fields.get(format!("{prefix}cross").as_str()).copied();
    let (edge, raw) = match (rise, fall, cross) {
        (Some(v), None, None) => (Edge::Rising, v),
        (None, Some(v), None) => (Edge::Falling, v),
        (None, None, Some(v)) => (Edge::Either, v),
        (None, None, None) => (Edge::Rising, "1"),
        _ => {
            return Err(format!(
                "'{prefix}rise'/'{prefix}fall'/'{prefix}cross' are mutually exclusive"
            ))
        }
    };
    let occurrence = if raw.eq_ignore_ascii_case("last") {
        Occurrence::Last
    } else {
        let n: usize = raw.parse().map_err(|_| {
            format!(
                "'{prefix}rise'/'{prefix}fall'/'{prefix}cross' value must be a positive integer \
                 or 'last', got '{raw}'"
            )
        })?;
        Occurrence::Nth(n)
    };
    Ok(CrossingCfg { edge, occurrence })
}

fn get_str(fields: &BTreeMap<&str, &str>, key: &str) -> Result<String, String> {
    fields
        .get(key)
        .map(|s| s.to_string())
        .ok_or_else(|| format!("missing field '{key}'"))
}

fn get_f64(fields: &BTreeMap<&str, &str>, key: &str) -> Result<f64, String> {
    get_str(fields, key)?
        .parse::<f64>()
        .map_err(|_| format!("field '{key}' is not a number"))
}

fn get_f64_opt(fields: &BTreeMap<&str, &str>, key: &str) -> Result<Option<f64>, String> {
    match fields.get(key) {
        Some(v) => Ok(Some(
            v.parse::<f64>()
                .map_err(|_| format!("field '{key}' is not a number"))?,
        )),
        None => Ok(None),
    }
}

/// Extracts every real (`t`, `value`) sample of `name` from the resolved waveform. `name` must
/// match a header exactly — `V(<node>)` for a node voltage, a declared block's own name (or
/// `<name>[<i>]` for a vector-valued block output) otherwise, the same convention the CSV/raw
/// output already uses.
fn samples_of(headers: &[String], rows: &[Vec<f64>], name: &str) -> Result<Vec<Sample>, String> {
    let idx = headers.iter().position(|h| h == name).ok_or_else(|| {
        format!(
            "unknown signal '{name}' (not a column of the resolved waveform — expected \
             'V(<node>)' for a node voltage, or a declared block's own name)"
        )
    })?;
    Ok(rows.iter().map(|r| Sample::new(r[0], r[idx])).collect())
}

fn resolve_event_cfg(cfg: &EventCfg, headers: &[String], rows: &[Vec<f64>]) -> Result<f64, String> {
    match cfg {
        EventCfg::At(t) => Ok(*t),
        EventCfg::Crossing {
            var,
            threshold,
            window,
            crossing,
        } => {
            let samples = samples_of(headers, rows, var)?;
            let thr_samples;
            let thr = match threshold {
                ThresholdCfg::Value(v) => Threshold::Value(*v),
                ThresholdCfg::Ref(r) => {
                    thr_samples = samples_of(headers, rows, r)?;
                    Threshold::Variable(&thr_samples)
                }
            };
            let spec = EventSpec::Crossing {
                var: &samples,
                threshold: thr,
                window: *window,
                edge: crossing.edge,
                occurrence: crossing.occurrence,
            };
            resolve_event(&spec).map_err(|e| e.to_string())
        }
        EventCfg::FracMax {
            var,
            frac,
            window,
            crossing,
        } => {
            let samples = samples_of(headers, rows, var)?;
            let spec = EventSpec::FracMax {
                var: &samples,
                frac: *frac,
                window: *window,
                edge: crossing.edge,
                occurrence: crossing.occurrence,
            };
            resolve_event(&spec).map_err(|e| e.to_string())
        }
    }
}

/// One measurement's outcome: its own `name` (redundant with the key inside `Ok`'s labels for
/// the common single-result case, but always present so a failed measurement can still be
/// reported by name) paired with either its `(label, value)` results or an error message.
type MeasureOutcome = (String, Result<Vec<(String, f64)>, String>);

/// Evaluates every collected measurement against the completed `(headers, rows)` waveform
/// (`headers[0]` is always `"t"`, exactly as built for CSV/raw output), returning one or more
/// `(label, value)` results per spec — most measurement types produce exactly one, `four`
/// produces one per harmonic plus (when at least 3 harmonics were requested) a THD line. A
/// failure on one measurement (an unknown signal name, an out-of-window crossing that never
/// occurs, ...) does not abort the others — each spec's own `Result` is reported independently.
pub fn evaluate_all(
    specs: &[MeasureSpec],
    headers: &[String],
    rows: &[Vec<f64>],
) -> Vec<MeasureOutcome> {
    specs
        .iter()
        .map(|spec| (spec.name.clone(), evaluate_one(spec, headers, rows)))
        .collect()
}

fn evaluate_one(
    spec: &MeasureSpec,
    headers: &[String],
    rows: &[Vec<f64>],
) -> Result<Vec<(String, f64)>, String> {
    let name = &spec.name;
    let wrap = |e: gs_waveform_measurements::MeasureError| -> String {
        format!("line {}: measurement '{name}': {e}", spec.line)
    };
    let result = match &spec.kind {
        MeasureKind::Max { out, window } => {
            let s = samples_of(headers, rows, out)?;
            vec![(name.clone(), max(&s, window).map_err(wrap)?.value)]
        }
        MeasureKind::Min { out, window } => {
            let s = samples_of(headers, rows, out)?;
            vec![(name.clone(), min(&s, window).map_err(wrap)?.value)]
        }
        MeasureKind::MaxAt { out, window } => {
            let s = samples_of(headers, rows, out)?;
            vec![(name.clone(), max(&s, window).map_err(wrap)?.time)]
        }
        MeasureKind::MinAt { out, window } => {
            let s = samples_of(headers, rows, out)?;
            vec![(name.clone(), min(&s, window).map_err(wrap)?.time)]
        }
        MeasureKind::Pp { out, window } => {
            let s = samples_of(headers, rows, out)?;
            vec![(name.clone(), peak_to_peak(&s, window).map_err(wrap)?)]
        }
        MeasureKind::Avg { out, window } => {
            let s = samples_of(headers, rows, out)?;
            vec![(name.clone(), avg(&s, window).map_err(wrap)?)]
        }
        MeasureKind::Rms { out, window } => {
            let s = samples_of(headers, rows, out)?;
            vec![(name.clone(), rms(&s, window).map_err(wrap)?)]
        }
        MeasureKind::Integ { out, window } => {
            let s = samples_of(headers, rows, out)?;
            vec![(name.clone(), integ(&s, window).map_err(wrap)?)]
        }
        MeasureKind::DerivAt { out, at } => {
            let s = samples_of(headers, rows, out)?;
            vec![(name.clone(), deriv_at(&s, *at).map_err(wrap)?)]
        }
        MeasureKind::DerivWhen {
            out,
            when_var,
            threshold,
            window,
            crossing,
        } => {
            let variable = samples_of(headers, rows, out)?;
            let when_samples = samples_of(headers, rows, when_var)?;
            let thr_samples;
            let thr = match threshold {
                ThresholdCfg::Value(v) => Threshold::Value(*v),
                ThresholdCfg::Ref(r) => {
                    thr_samples = samples_of(headers, rows, r)?;
                    Threshold::Variable(&thr_samples)
                }
            };
            let v = deriv_when(
                &variable,
                &when_samples,
                thr,
                window,
                crossing.edge,
                crossing.occurrence,
            )
            .map_err(wrap)?;
            vec![(name.clone(), v)]
        }
        MeasureKind::FindAt { out, at } => {
            let s = samples_of(headers, rows, out)?;
            vec![(name.clone(), find_at(&s, *at).map_err(wrap)?)]
        }
        MeasureKind::FindWhen {
            out,
            when_var,
            threshold,
            window,
            crossing,
        } => {
            let find_var = samples_of(headers, rows, out)?;
            let when_samples = samples_of(headers, rows, when_var)?;
            let thr_samples;
            let thr = match threshold {
                ThresholdCfg::Value(v) => Threshold::Value(*v),
                ThresholdCfg::Ref(r) => {
                    thr_samples = samples_of(headers, rows, r)?;
                    Threshold::Variable(&thr_samples)
                }
            };
            let v = find_when(
                &find_var,
                &when_samples,
                thr,
                window,
                crossing.edge,
                crossing.occurrence,
            )
            .map_err(wrap)?;
            vec![(name.clone(), v)]
        }
        MeasureKind::When {
            out,
            threshold,
            window,
            crossing,
        } => {
            let var_samples = samples_of(headers, rows, out)?;
            let thr_samples;
            let thr = match threshold {
                ThresholdCfg::Value(v) => Threshold::Value(*v),
                ThresholdCfg::Ref(r) => {
                    thr_samples = samples_of(headers, rows, r)?;
                    Threshold::Variable(&thr_samples)
                }
            };
            let t = when(
                &var_samples,
                thr,
                window,
                crossing.edge,
                crossing.occurrence,
            )
            .map_err(wrap)?;
            vec![(name.clone(), t)]
        }
        MeasureKind::TrigTarg { trig, targ } => {
            let t_trig = resolve_event_cfg(trig, headers, rows)
                .map_err(|e| format!("line {}: measurement '{name}': trig: {e}", spec.line))?;
            let t_targ = resolve_event_cfg(targ, headers, rows)
                .map_err(|e| format!("line {}: measurement '{name}': targ: {e}", spec.line))?;
            vec![(name.clone(), t_targ - t_trig)]
        }
        MeasureKind::Freq {
            out,
            on,
            off,
            window,
        } => {
            let s = samples_of(headers, rows, out)?;
            vec![(name.clone(), freq(&s, window, *on, *off).map_err(wrap)?)]
        }
        MeasureKind::OnTime {
            out,
            on,
            off,
            window,
        } => {
            let s = samples_of(headers, rows, out)?;
            vec![(name.clone(), on_time(&s, window, *on, *off).map_err(wrap)?)]
        }
        MeasureKind::OffTime {
            out,
            on,
            off,
            window,
        } => {
            let s = samples_of(headers, rows, out)?;
            vec![(name.clone(), off_time(&s, window, *on, *off).map_err(wrap)?)]
        }
        MeasureKind::Four {
            out,
            fundamental,
            harmonics,
            window,
        } => {
            let s = samples_of(headers, rows, out)?;
            let result = four(&s, window, *fundamental, *harmonics).map_err(wrap)?;
            let mut out_vals = Vec::new();
            for h in &result.harmonics {
                if h.order == 0 {
                    out_vals.push((format!("{name}_dc"), h.magnitude));
                } else {
                    out_vals.push((format!("{name}_h{}_mag", h.order), h.magnitude));
                    out_vals.push((format!("{name}_h{}_phase_deg", h.order), h.phase_degrees));
                }
            }
            if *harmonics >= 3 {
                out_vals.push((format!("{name}_thd_percent"), result.thd_percent));
            }
            out_vals
        }
        MeasureKind::Err1 {
            out,
            reference,
            minval,
            ymin,
            ymax,
        } => {
            let measured = samples_of(headers, rows, out)?;
            let comparison = samples_of(headers, rows, reference)?;
            let v = err1(&measured, &comparison, *minval, *ymin, *ymax).map_err(wrap)?;
            vec![(name.clone(), v)]
        }
        MeasureKind::Err2 {
            out,
            reference,
            minval,
            ymin,
            ymax,
        } => {
            let measured = samples_of(headers, rows, out)?;
            let comparison = samples_of(headers, rows, reference)?;
            let v = err2(&measured, &comparison, *minval, *ymin, *ymax).map_err(wrap)?;
            vec![(name.clone(), v)]
        }
        MeasureKind::Error {
            out,
            reference,
            norm,
        } => {
            let measured = samples_of(headers, rows, out)?;
            let reference_samples = samples_of(headers, rows, reference)?;
            let v = error_norm(&measured, &reference_samples, *norm).map_err(wrap)?;
            vec![(name.clone(), v)]
        }
    };
    Ok(result)
}
