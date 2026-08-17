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
//! `--mode closed-loop` additionally recognizes two more device-file line kinds (only usable
//! together, and only one `kind=pid` controller per file -- `dae_runtime::simulate_closed_loop`
//! itself is single-controller):
//!
//! ```text
//! CTRL1 kind=pid measure=vout kp=800 ki=4e6 kd=0 n=1000 f_nom=115000 f_min=100000 f_max=130000 duty=0.48
//! D1 kind=mosfet r_on=0.01 g_breakdown=0 v_breakdown=-1e6 g_off=1e-6 v_th=1e6 g_on=0 gate=freqpid ctrl=CTRL1 phase=0
//! D2 kind=mosfet r_on=0.01 g_breakdown=0 v_breakdown=-1e6 g_off=1e-6 v_th=1e6 g_on=0 gate=freqpid ctrl=CTRL1 phase=0.5
//! REF0 kind=ref ctrl=CTRL1 t=0 v=20
//! REF1 kind=ref ctrl=CTRL1 t=0.012 v=15
//! ```
//!
//! `kind=pid` compiles a `continuous_blocks::Pid` and drives every `gate=freqpid` MOSFET from a
//! single shared numerically-controlled oscillator: `freq = (f_nom - pid_output).clamp(f_min,
//! f_max)`, phase accumulated each step by `freq*dt`, each MOSFET on while `(phase +
//! device_phase).rem_euclid(1.0) < duty` -- a frequency-modulated half-bridge/PWM, as opposed
//! to `gate=pwm`'s fixed-frequency duty modulation (buck/boost use duty; LLC-family resonant
//! converters are controlled by frequency instead, see
//! `internal-archive/experiments/elspice-pwl-llc-closed-loop-vs-xyce-ngspice/`).
//! `kind=ref` entries (any number, one `t=`/`v=` pair each) build that controller's
//! piecewise-constant reference-vs-time schedule -- a reference *step* test (settle at one
//! setpoint, then jump to another) is just two `kind=ref` lines, not a separate code path.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::process::ExitCode;

use continuous_blocks::Pid;
use dae_runtime::{
    simulate_closed_loop, simulate_transient, simulate_transient_with_mosfets, solve_dc,
    solve_dc_with_mosfets, GateState,
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
    Pid(PidConfig),
    Ref {
        ctrl: String,
        t: f64,
        v: f64,
    },
}

#[derive(Clone)]
struct PidConfig {
    kp: f64,
    ki: f64,
    kd: f64,
    n: f64,
    measure: String,
    f_nom: f64,
    f_min: f64,
    f_max: f64,
    duty: f64,
}

#[derive(Clone)]
enum GateSpec {
    Fixed(GateState),
    Pwm { freq_hz: f64, duty: f64 },
    FreqPid { ctrl: String, phase: f64 },
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
    let mut controllers: BTreeMap<String, PidConfig> = BTreeMap::new();
    let mut ref_schedule: Vec<(String, f64, f64)> = Vec::new();
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
            Kind::Pid(config) => {
                controllers.insert(name, config);
            }
            Kind::Ref { ctrl, t, v } => {
                ref_schedule.push((ctrl, t, v));
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
                .map(|(n, (_, g))| (n.clone(), g.clone()))
                .collect::<BTreeMap<_, _>>();
            for (name, gate) in &gates {
                if matches!(gate, GateSpec::FreqPid { .. }) {
                    return Err(format!(
                        "device '{name}': gate=freqpid needs --mode closed-loop (a fixed/pwm-scheduled \
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
                Some(GateSpec::FreqPid { .. }) | None => GateState::Off,
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
            &controllers,
            &ref_schedule,
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
                GateSpec::FreqPid { .. } => {
                    return Err(format!(
                        "device '{name}': gate=freqpid needs --mode closed-loop, not 'dc'"
                    ))
                }
            };
            Ok((name.clone(), (*m, state)))
        })
        .collect()
}

/// `--mode closed-loop`: drives every `gate=freqpid` MOSFET from a single shared
/// numerically-controlled oscillator under one `kind=pid` controller's command -- see this
/// file's module doc comment for the device-file syntax.
#[allow(clippy::too_many_arguments)]
fn run_closed_loop(
    netlist: &str,
    dialect: Dialect,
    diodes: &BTreeMap<String, Diode>,
    mosfets: &BTreeMap<String, (Mosfet, GateSpec)>,
    controllers: &BTreeMap<String, PidConfig>,
    ref_schedule: &[(String, f64, f64)],
    shared_r_on: f64,
    t_final: f64,
    dt: f64,
) -> Result<(), String> {
    if controllers.len() != 1 {
        return Err(format!(
            "--mode closed-loop needs exactly one kind=pid controller in the devices file, found {}",
            controllers.len()
        ));
    }
    let (ctrl_name, config) = controllers.iter().next().unwrap();

    let mosfets_only: BTreeMap<String, Mosfet> =
        mosfets.iter().map(|(n, (m, _))| (n.clone(), *m)).collect();
    let mut phases: BTreeMap<String, f64> = BTreeMap::new();
    for (name, (_, gate)) in mosfets {
        match gate {
            GateSpec::FreqPid { ctrl, phase } => {
                if ctrl != ctrl_name {
                    return Err(format!(
                        "device '{name}': gate=freqpid references unknown controller '{ctrl}'"
                    ));
                }
                phases.insert(name.clone(), *phase);
            }
            _ => {
                return Err(format!(
                    "device '{name}': --mode closed-loop needs every MOSFET to use gate=freqpid"
                ))
            }
        }
    }
    if phases.is_empty() {
        return Err("--mode closed-loop needs at least one gate=freqpid MOSFET".to_string());
    }

    let mut schedule: Vec<(f64, f64)> = ref_schedule
        .iter()
        .filter(|(ctrl, _, _)| ctrl == ctrl_name)
        .map(|(_, t, v)| (*t, *v))
        .collect();
    if schedule.is_empty() {
        return Err(format!(
            "controller '{ctrl_name}' has no kind=ref entries (need at least one reference point)"
        ));
    }
    schedule.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let reference = move |t: f64| -> f64 {
        let mut v = schedule[0].1;
        for &(t_i, v_i) in &schedule {
            if t_i <= t {
                v = v_i;
            } else {
                break;
            }
        }
        v
    };

    let pid = Pid::new(config.kp, config.ki, config.kd, config.n);
    let controller = pid.to_state_space();
    let measure_expr = format!("V({})", config.measure);
    let measure = move |point: &dae_runtime::OperatingPoint| point.value(&measure_expr).unwrap();

    let f_nom = config.f_nom;
    let f_min = config.f_min;
    let f_max = config.f_max;
    let duty = config.duty;
    let phase_acc = Cell::new(0.0f64);
    let pwm = move |pid_output: f64, _t: f64| {
        let freq = (f_nom - pid_output).clamp(f_min, f_max);
        let p = (phase_acc.get() + freq * dt).rem_euclid(1.0);
        phase_acc.set(p);
        phases
            .iter()
            .map(|(name, offset)| {
                let local = (p + offset).rem_euclid(1.0);
                let state = if local < duty {
                    GateState::On
                } else {
                    GateState::Off
                };
                (name.clone(), state)
            })
            .collect()
    };
    let output_clamp = (f_nom - f_max, f_nom - f_min);

    let trace = simulate_closed_loop(
        netlist,
        dialect,
        diodes,
        &mosfets_only,
        &controller,
        reference,
        measure,
        pwm,
        output_clamp,
        shared_r_on,
        None,
        t_final,
        dt,
    )
    .map_err(|e| format!("{e:?}"))?;

    if let Some((_, first, _)) = trace.first() {
        println!("t,{},freq", first.unknowns.join(","));
    }
    for (t, point, pid_output) in &trace {
        let values: Vec<String> = point.x.iter().map(|v| v.to_string()).collect();
        let freq = f_nom - pid_output;
        println!("{t},{},{freq}", values.join(","));
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
                    Some("freqpid") => GateSpec::FreqPid {
                        ctrl: get_str("ctrl")?,
                        phase: get("phase")?,
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
            "pid" => Kind::Pid(PidConfig {
                kp: get("kp")?,
                ki: get("ki")?,
                kd: get("kd")?,
                n: get("n")?,
                measure: get_str("measure")?,
                f_nom: get("f_nom")?,
                f_min: get("f_min")?,
                f_max: get("f_max")?,
                duty: get("duty")?,
            }),
            "ref" => Kind::Ref {
                ctrl: get_str("ctrl")?,
                t: get("t")?,
                v: get("v")?,
            },
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
    "usage: elspice-pwl <netlist> [--devices <file>] [--mode dc|transient|closed-loop] \
     [--tfinal T] [--dt DT]"
        .to_string()
}
