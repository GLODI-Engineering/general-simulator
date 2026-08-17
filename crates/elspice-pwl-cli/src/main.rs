//! Thin runner for `elspice-pwl`: `elspice-pwl <netlist> --devices <devices-file> --mode
//! {dc|transient|closed-loop} [--tfinal T --dt DT]` prints a CSV waveform
//! (`t,V(node1),V(node2),...`) to stdout, one row per resolved timestep (a single row for
//! `--mode dc`).
//!
//! The devices file is a small hand-rolled `key=value` format (no serde/TOML dependency needed
//! for something this simple), one line per PWL device, first token the element name:
//!
//! ```text
//! D1 kind=diode g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.7 g_on=1
//! D2 kind=mosfet r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.5 g_on=5 gate=on
//! D3 kind=mosfet r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.5 g_on=5 gate=pwm freq=10000 duty=0.6
//! ```
//!
//! `#` or `;` starts a comment; blank lines are skipped. Every MOSFET must declare the same
//! `r_on` — `dae-runtime`'s switch mechanism uses one shared on-resistance per call (see
//! `dae_runtime::solve_dc_with_mosfets`'s doc comment).
//!
//! ## `--mode closed-loop`
//!
//! A closed loop is a graph of small, independently reusable `continuous-blocks` blocks —
//! `const`, `pwl` (piecewise-constant source, e.g. a reference schedule), `sum` (an error
//! junction with explicit `+`/`-` signs), `gain`, `pid`, `vco` — wired together with plain
//! device-file lines, the same discipline a real block-diagram tool (a reference tool, a reference tool) uses:
//! **the error signal is a `sum` block's own output, and a frequency-modulated PWM carrier is
//! `sum -> pid -> sum -> vco`, not one fused "closed-loop controller" that bakes a specific
//! topology together.** Each block is declared with `kind=<block>`, its own parameters, and
//! `in=<signal>` (single-input blocks) or `inputs=<signal>,<signal>,...` (`sum`, one per
//! `signs=` entry). A `<signal>` is either another block's name (its output this same step) or
//! `meas:<node>` (the circuit's own previous-step measurement, e.g. `meas:vout` for
//! `V(vout)`). **Blocks are evaluated in the order they appear in the file** — every signal
//! must reference a block declared *earlier* (or a `meas:` signal, which has no ordering
//! constraint) — so declare sources first and sinks last, same as you'd read a signal-flow
//! diagram left to right.
//!
//! A MOSFET's gate can reference a `kind=vco` block by name instead of a fixed/`pwm` spec:
//! `gate=vco ctrl=<vco-block-name> phase=<0..1> duty=<0..1>` — several gates naming the same
//! `vco` share one oscillator with different phase offsets (a half-bridge's two complementary
//! switches, for instance) rather than needing a separate oscillator per gate.
//!
//! Example — a frequency-modulated half-bridge PID (LLC-family converters regulate by
//! switching frequency, not PWM duty, unlike buck/boost) with a reference step test:
//!
//! ```text
//! REF    kind=pwl points=0:20,0.014:17
//! ERR    kind=sum inputs=REF,meas:vout signs=1,-1
//! PID1   kind=pid kp=800 ki=4e6 kd=0 n=1000 clamp_lo=-15000 clamp_hi=15000 in=ERR
//! FNOM   kind=const value=115000
//! FREQ   kind=sum inputs=FNOM,PID1 signs=1,-1
//! VCO1   kind=vco f_min=100000 f_max=130000 in=FREQ
//!
//! D1 kind=mosfet r_on=0.01 g_breakdown=0 v_breakdown=-1e6 g_off=1e-6 v_th=1e6 g_on=0 gate=vco ctrl=VCO1 phase=0 duty=0.48
//! D2 kind=mosfet r_on=0.01 g_breakdown=0 v_breakdown=-1e6 g_off=1e-6 v_th=1e6 g_on=0 gate=vco ctrl=VCO1 phase=0.5 duty=0.48
//! ```
//!
//! See `internal-archive/experiments/elspice-pwl-llc-closed-loop-vs-xyce-ngspice/`
//! for the full worked example this syntax was built for.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::process::ExitCode;

use continuous_blocks::{Pid, Vco};
use dae_runtime::{
    simulate_closed_loop_blocks, simulate_transient, simulate_transient_with_mosfets, solve_dc,
    solve_dc_with_mosfets, BlockInstance, BlockKind, GateBinding, GateState, Signal,
};
use pwl_devices::{Diode, Mosfet};
use spice_core::Dialect;

enum Kind {
    Diode(Diode),
    Mosfet {
        mosfet: Mosfet,
        r_on: f64,
        gate: GateSpec,
    },
    Block(BlockInstance),
}

#[derive(Clone)]
enum GateSpec {
    Fixed(GateState),
    Pwm { freq_hz: f64, duty: f64 },
    Vco { ctrl: String, phase: f64, duty: f64 },
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        return Err(usage());
    }
    let netlist_path = &args[1];
    let mut devices_path: Option<String> = None;
    let mut mode = "dc".to_string();
    let mut t_final = 1.0;
    let mut dt = 1e-3;

    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--devices" => {
                devices_path = Some(args.get(i + 1).ok_or("--devices needs a value")?.clone());
                i += 2;
            }
            "--mode" => {
                mode = args.get(i + 1).ok_or("--mode needs a value")?.clone();
                i += 2;
            }
            "--tfinal" => {
                t_final = args
                    .get(i + 1)
                    .ok_or("--tfinal needs a value")?
                    .parse()
                    .map_err(|_| "bad --tfinal")?;
                i += 2;
            }
            "--dt" => {
                dt = args
                    .get(i + 1)
                    .ok_or("--dt needs a value")?
                    .parse()
                    .map_err(|_| "bad --dt")?;
                i += 2;
            }
            other => return Err(format!("unrecognized argument '{other}'\n{}", usage())),
        }
    }

    let netlist =
        fs::read_to_string(netlist_path).map_err(|e| format!("reading {netlist_path}: {e}"))?;
    let devices = match &devices_path {
        Some(path) => {
            let text = fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
            parse_devices(&text)?
        }
        None => Vec::new(),
    };

    let mut diodes = BTreeMap::new();
    let mut mosfets = BTreeMap::new();
    let mut blocks: Vec<BlockInstance> = Vec::new();
    let mut shared_r_on: Option<f64> = None;
    for (name, kind) in devices {
        match kind {
            Kind::Diode(d) => {
                diodes.insert(name, d);
            }
            Kind::Mosfet { mosfet, r_on, gate } => {
                match shared_r_on {
                    None => shared_r_on = Some(r_on),
                    Some(existing) if (existing - r_on).abs() > 1e-15 => {
                        return Err(format!(
                            "all MOSFETs must share the same r_on (dae-runtime's switch model uses one \
                             shared on-resistance per call); got {existing} and {r_on}"
                        ));
                    }
                    Some(_) => {}
                }
                mosfets.insert(name, (mosfet, gate));
            }
            Kind::Block(instance) => blocks.push(instance),
        }
    }
    let shared_r_on = shared_r_on.unwrap_or(0.0);

    let dialect = Dialect::Ngspice;

    if mode == "dc" {
        let point = if mosfets.is_empty() {
            solve_dc(&netlist, dialect, &diodes).map_err(|e| format!("{e:?}"))?
        } else {
            let mosfets_fixed = fixed_gate_states(&mosfets, 0.0)?;
            solve_dc_with_mosfets(&netlist, dialect, &diodes, &mosfets_fixed, shared_r_on)
                .map_err(|e| format!("{e:?}"))?
        };
        print_header(&point.unknowns);
        print_row(0.0, &point);
    } else if mode == "transient" {
        if mosfets.is_empty() {
            let trace = simulate_transient(&netlist, dialect, &diodes, None, t_final, dt)
                .map_err(|e| format!("{e:?}"))?;
            if let Some((_, first)) = trace.first() {
                print_header(&first.unknowns);
            }
            for (t, point) in &trace {
                print_row(*t, point);
            }
        } else {
            let mosfets_only: BTreeMap<String, Mosfet> =
                mosfets.iter().map(|(n, (m, _))| (n.clone(), *m)).collect();
            let gates = mosfets
                .iter()
                .map(|(n, (_, g))| (n.clone(), g.clone()))
                .collect::<BTreeMap<_, _>>();
            for (name, gate) in &gates {
                if matches!(gate, GateSpec::Vco { .. }) {
                    return Err(format!(
                        "device '{name}': gate=vco needs --mode closed-loop (a fixed/pwm-scheduled \
                         --mode transient run has no controller to drive the oscillator's frequency)"
                    ));
                }
            }
            let gate_signal = move |name: &str, t: f64| match gates.get(name) {
                Some(GateSpec::Fixed(state)) => *state,
                Some(GateSpec::Pwm { freq_hz, duty }) => {
                    let phase = t * freq_hz;
                    let carrier = phase - phase.floor();
                    if carrier < *duty {
                        GateState::On
                    } else {
                        GateState::Off
                    }
                }
                Some(GateSpec::Vco { .. }) | None => GateState::Off,
            };
            let trace = simulate_transient_with_mosfets(
                &netlist,
                dialect,
                &diodes,
                &mosfets_only,
                gate_signal,
                shared_r_on,
                None,
                t_final,
                dt,
            )
            .map_err(|e| format!("{e:?}"))?;
            if let Some((_, first)) = trace.first() {
                print_header(&first.unknowns);
            }
            for (t, point) in &trace {
                print_row(*t, point);
            }
        }
    } else if mode == "closed-loop" {
        run_closed_loop(
            &netlist,
            dialect,
            &diodes,
            &mosfets,
            &blocks,
            shared_r_on,
            t_final,
            dt,
        )?;
    } else {
        return Err(format!(
            "unknown --mode '{mode}' (expected 'dc', 'transient', or 'closed-loop')"
        ));
    }

    Ok(())
}

fn fixed_gate_states(
    mosfets: &BTreeMap<String, (Mosfet, GateSpec)>,
    t: f64,
) -> Result<BTreeMap<String, (Mosfet, GateState)>, String> {
    mosfets
        .iter()
        .map(|(name, (m, gate))| {
            let state = match gate {
                GateSpec::Fixed(s) => *s,
                GateSpec::Pwm { freq_hz, duty } => {
                    let phase = t * freq_hz;
                    let carrier = phase - phase.floor();
                    if carrier < *duty {
                        GateState::On
                    } else {
                        GateState::Off
                    }
                }
                GateSpec::Vco { .. } => {
                    return Err(format!(
                        "device '{name}': gate=vco needs --mode closed-loop, not 'dc'"
                    ))
                }
            };
            Ok((name.clone(), (*m, state)))
        })
        .collect()
}

/// `--mode closed-loop`: runs the device file's `kind=const/pwl/sum/gain/pid/vco` block graph
/// alongside the circuit, driving every `gate=vco` MOSFET from its named oscillator block —
/// see this file's module doc comment for the full syntax and an LLC-converter example.
#[allow(clippy::too_many_arguments)]
fn run_closed_loop(
    netlist: &str,
    dialect: Dialect,
    diodes: &BTreeMap<String, Diode>,
    mosfets: &BTreeMap<String, (Mosfet, GateSpec)>,
    blocks: &[BlockInstance],
    shared_r_on: f64,
    t_final: f64,
    dt: f64,
) -> Result<(), String> {
    if blocks.is_empty() {
        return Err(
            "--mode closed-loop needs at least one block (kind=const/pwl/sum/gain/pid/vco) \
             in the devices file"
                .to_string(),
        );
    }

    let mosfets_only: BTreeMap<String, Mosfet> =
        mosfets.iter().map(|(n, (m, _))| (n.clone(), *m)).collect();
    let mut gates: BTreeMap<String, GateBinding> = BTreeMap::new();
    for (name, (_, gate)) in mosfets {
        match gate {
            GateSpec::Vco { ctrl, phase, duty } => {
                gates.insert(
                    name.clone(),
                    GateBinding {
                        vco: ctrl.clone(),
                        phase: *phase,
                        duty: *duty,
                    },
                );
            }
            _ => {
                return Err(format!(
                    "device '{name}': --mode closed-loop needs every MOSFET to use gate=vco"
                ))
            }
        }
    }
    if gates.is_empty() {
        return Err("--mode closed-loop needs at least one gate=vco MOSFET".to_string());
    }

    let trace = simulate_closed_loop_blocks(
        netlist,
        dialect,
        diodes,
        &mosfets_only,
        blocks,
        &gates,
        shared_r_on,
        None,
        t_final,
        dt,
    )
    .map_err(|e| format!("{e:?}"))?;

    let block_names: Vec<String> = blocks.iter().map(|b| b.name.clone()).collect();
    if let Some((_, first, _)) = trace.first() {
        println!("t,{},{}", first.unknowns.join(","), block_names.join(","));
    }
    for (t, point, outputs) in &trace {
        let values: Vec<String> = point.x.iter().map(|v| v.to_string()).collect();
        let block_values: Vec<String> = block_names
            .iter()
            .map(|name| outputs.get(name).copied().unwrap_or(f64::NAN).to_string())
            .collect();
        println!("{t},{},{}", values.join(","), block_values.join(","));
    }

    Ok(())
}

fn print_header(unknowns: &[String]) {
    println!("t,{}", unknowns.join(","));
}

fn print_row(t: f64, point: &dae_runtime::OperatingPoint) {
    let values: Vec<String> = point.x.iter().map(|v| v.to_string()).collect();
    println!("{t},{}", values.join(","));
}

/// Parses a `<signal>` field value: `meas:<node>` for a circuit measurement, anything else is
/// another block's name.
fn parse_signal(text: &str) -> Signal {
    match text.strip_prefix("meas:") {
        Some(node) => Signal::Measure(node.to_string()),
        None => Signal::Block(text.to_string()),
    }
}

fn parse_devices(text: &str) -> Result<Vec<(String, Kind)>, String> {
    let mut result = Vec::new();
    for (line_number, raw_line) in text.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        let mut tokens = line.split_whitespace();
        let name = tokens
            .next()
            .ok_or_else(|| format!("line {}: missing device name", line_number + 1))?;
        let fields: BTreeMap<String, String> = tokens
            .map(|token| {
                let mut parts = token.splitn(2, '=');
                let key = parts.next().unwrap_or_default().to_string();
                let value = parts.next().unwrap_or_default().to_string();
                (key, value)
            })
            .collect();

        let get = |key: &str| -> Result<f64, String> {
            fields
                .get(key)
                .ok_or_else(|| {
                    format!(
                        "line {}: device '{name}' missing field '{key}'",
                        line_number + 1
                    )
                })?
                .parse::<f64>()
                .map_err(|_| {
                    format!(
                        "line {}: device '{name}' field '{key}' is not a number",
                        line_number + 1
                    )
                })
        };

        let get_str = |key: &str| -> Result<String, String> {
            fields.get(key).cloned().ok_or_else(|| {
                format!(
                    "line {}: device '{name}' missing field '{key}'",
                    line_number + 1
                )
            })
        };

        let kind = fields.get("kind").map(String::as_str).unwrap_or("diode");
        let entry = match kind {
            "diode" => Kind::Diode(Diode::new(
                get("g_breakdown")?,
                get("v_breakdown")?,
                get("g_off")?,
                get("v_th")?,
                get("g_on")?,
            )),
            "mosfet" => {
                let body_diode = Diode::new(
                    get("g_breakdown")?,
                    get("v_breakdown")?,
                    get("g_off")?,
                    get("v_th")?,
                    get("g_on")?,
                );
                let r_on = get("r_on")?;
                let gate = match fields.get("gate").map(String::as_str) {
                    Some("on") => GateSpec::Fixed(GateState::On),
                    Some("off") | None => GateSpec::Fixed(GateState::Off),
                    Some("pwm") => GateSpec::Pwm {
                        freq_hz: get("freq")?,
                        duty: get("duty")?,
                    },
                    Some("vco") => GateSpec::Vco {
                        ctrl: get_str("ctrl")?,
                        phase: get("phase")?,
                        duty: get("duty")?,
                    },
                    Some(other) => {
                        return Err(format!(
                            "line {}: unknown gate spec '{other}'",
                            line_number + 1
                        ))
                    }
                };
                Kind::Mosfet {
                    mosfet: Mosfet::new(r_on, body_diode),
                    r_on,
                    gate,
                }
            }
            "const" => Kind::Block(BlockInstance {
                name: name.to_string(),
                kind: BlockKind::Const(get("value")?),
                inputs: Vec::new(),
            }),
            "pwl" => {
                let points_text = get_str("points")?;
                let mut points = Vec::new();
                for point in points_text.split(',') {
                    let (t_str, v_str) = point.split_once(':').ok_or_else(|| {
                        format!(
                            "line {}: device '{name}' field 'points' entry '{point}' is not \
                             '<t>:<v>'",
                            line_number + 1
                        )
                    })?;
                    let t: f64 = t_str.parse().map_err(|_| {
                        format!(
                            "line {}: device '{name}' points entry '{point}': '{t_str}' is \
                             not a number",
                            line_number + 1
                        )
                    })?;
                    let v: f64 = v_str.parse().map_err(|_| {
                        format!(
                            "line {}: device '{name}' points entry '{point}': '{v_str}' is \
                             not a number",
                            line_number + 1
                        )
                    })?;
                    points.push((t, v));
                }
                points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
                Kind::Block(BlockInstance {
                    name: name.to_string(),
                    kind: BlockKind::Pwl(points),
                    inputs: Vec::new(),
                })
            }
            "sum" => {
                let inputs: Vec<Signal> = get_str("inputs")?.split(',').map(parse_signal).collect();
                let signs: Vec<f64> = get_str("signs")?
                    .split(',')
                    .map(|s| {
                        s.parse::<f64>().map_err(|_| {
                            format!(
                                "line {}: device '{name}' field 'signs' entry '{s}' is not a \
                                 number",
                                line_number + 1
                            )
                        })
                    })
                    .collect::<Result<_, _>>()?;
                if inputs.len() != signs.len() {
                    return Err(format!(
                        "line {}: device '{name}': 'inputs' has {} entries but 'signs' has {} \
                         (need one sign per input)",
                        line_number + 1,
                        inputs.len(),
                        signs.len()
                    ));
                }
                Kind::Block(BlockInstance {
                    name: name.to_string(),
                    kind: BlockKind::Sum(signs),
                    inputs,
                })
            }
            "gain" => Kind::Block(BlockInstance {
                name: name.to_string(),
                kind: BlockKind::Gain(get("k")?),
                inputs: vec![parse_signal(&get_str("in")?)],
            }),
            "pid" => {
                let pid = Pid::new(get("kp")?, get("ki")?, get("kd")?, get("n")?);
                let clamp = (get("clamp_lo")?, get("clamp_hi")?);
                Kind::Block(BlockInstance {
                    name: name.to_string(),
                    kind: BlockKind::Pid { pid, clamp },
                    inputs: vec![parse_signal(&get_str("in")?)],
                })
            }
            "vco" => {
                let vco = Vco::new(get("f_min")?, get("f_max")?);
                Kind::Block(BlockInstance {
                    name: name.to_string(),
                    kind: BlockKind::Vco(vco),
                    inputs: vec![parse_signal(&get_str("in")?)],
                })
            }
            other => {
                return Err(format!(
                    "line {}: unknown device kind '{other}'",
                    line_number + 1
                ))
            }
        };
        result.push((name.to_string(), entry));
    }
    Ok(result)
}

fn usage() -> String {
    "usage: elspice-pwl <netlist> [--devices <file>] [--mode dc|transient|closed-loop] \
     [--tfinal T] [--dt DT]"
        .to_string()
}
