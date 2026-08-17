//! Thin runner for `elspice-pwl`: `elspice-pwl <netlist> --devices <devices-file> --mode
//! {dc|transient} [--tfinal T --dt DT]` prints a CSV waveform (`t,V(node1),V(node2),...`) to
//! stdout, one row per resolved timestep (a single row for `--mode dc`).
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

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::process::ExitCode;

use dae_runtime::{
    simulate_transient, simulate_transient_with_mosfets, solve_dc, solve_dc_with_mosfets, GateState,
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
}

#[derive(Clone, Copy)]
enum GateSpec {
    Fixed(GateState),
    Pwm { freq_hz: f64, duty: f64 },
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
        None => BTreeMap::new(),
    };

    let mut diodes = BTreeMap::new();
    let mut mosfets = BTreeMap::new();
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
                .map(|(n, (_, g))| (n.clone(), *g))
                .collect::<BTreeMap<_, _>>();
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
                None => GateState::Off,
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
    } else {
        return Err(format!(
            "unknown --mode '{mode}' (expected 'dc' or 'transient')"
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
            };
            Ok((name.clone(), (*m, state)))
        })
        .collect()
}

fn print_header(unknowns: &[String]) {
    println!("t,{}", unknowns.join(","));
}

fn print_row(t: f64, point: &dae_runtime::OperatingPoint) {
    let values: Vec<String> = point.x.iter().map(|v| v.to_string()).collect();
    println!("{t},{}", values.join(","));
}

fn parse_devices(text: &str) -> Result<BTreeMap<String, Kind>, String> {
    let mut result = BTreeMap::new();
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
            other => {
                return Err(format!(
                    "line {}: unknown device kind '{other}'",
                    line_number + 1
                ))
            }
        };
        result.insert(name.to_string(), entry);
    }
    Ok(result)
}

fn usage() -> String {
    "usage: elspice-pwl <netlist> [--devices <file>] [--mode dc|transient] [--tfinal T] [--dt DT]"
        .to_string()
}
