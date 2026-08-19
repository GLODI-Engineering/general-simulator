//! Thin runner for `elspice-pwl`: `elspice-pwl <netlist> [--devices <file>] --mode
//! {dc|transient} [--tfinal T] [--dt DT | --dt-max/--dt-min/--dt-init/--reltol/--abstol]`
//! prints a CSV waveform (`t,V(node1),V(node2),...`) to stdout, one row per resolved timestep
//! (a single row for `--mode dc`). `--dt` fixes the step size; omit it for adaptive step-size
//! control instead (the default — see `dae_runtime::TimeStep`/`AdaptiveConfig`'s own doc
//! comments for the algorithm), with `--dt-max`/`--dt-min`/`--dt-init`/`--reltol`/`--abstol`
//! overriding individual defaults derived from `--tfinal`.
//!
//! **The netlist is the one file** — PWL device parameters and any controller block graph
//! live directly inside it, the same way a real SPICE deck is self-contained, not split across
//! files by convention. `spice-core` enforces real SPICE grammar (see its own docs), which has
//! no syntax for `kind=mosfet r_on=...` or a block graph, so those lines are written as
//! ordinary SPICE comments — anything starting with `*` — with a leading `kind=...` key/value
//! declaration (a small hand-rolled format, no serde/TOML dependency needed for something this
//! simple):
//!
//! ```text
//! * D1 kind=diode g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.7 g_on=1
//! * D2 kind=mosfet r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.5 g_on=5 gate=on
//! * D3 kind=mosfet r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.5 g_on=5 gate=pwm freq=10000 duty=0.6
//! ```
//!
//! Any other tool — a real SPICE simulator, a text editor, a diff — sees exactly what a `*`
//! line always means: an ordinary comment, safely ignored. `elspice-pwl-cli` is the only thing
//! that additionally reads these lines as device/block declarations (stripping the leading `*`;
//! a line without `kind=` — including a genuine comment that happens to start with `*` — is
//! left alone). This is the whole netlist: run `elspice-pwl-cli some.cir --mode transient` with
//! no `--devices` at all and it looks for these lines in `some.cir` itself. `--devices <file>`
//! remains available for the rarer case of sharing one controller/PWL-parameter file across
//! several netlists, but is not the default or the expected common case.
//!
//! `#`/`;`-prefixed lines (not `*`) and blank lines in either file are plain devices-file
//! comments, invisible to `elspice-pwl-cli` itself, not SPICE comments — use `*` for anything
//! meant to also survive being read by a real SPICE tool. Every MOSFET must declare the same
//! `r_on` — `dae-runtime`'s switch mechanism uses one shared on-resistance per call (see
//! `dae_runtime::solve_dc_with_mosfets`'s doc comment).
//!
//! ## Wiring a controller into a gate
//!
//! **There is no separate "closed-loop mode."** `--mode transient` always resolves every
//! MOSFET's gate the same way, every step, whether that gate is a fixed state, a fixed-
//! frequency/fixed-duty PWM schedule, or driven by a graph of `continuous-blocks` blocks the
//! device file wires together — `const`, `pwl` (piecewise-constant source, e.g. a reference
//! schedule), `sum` (an error junction with explicit `+`/`-` signs), `gain`, `pid`,
//! `statespace` (arbitrary `(A,B,C,D)`), `tf` (a rational `N(s)/D(s)`), `vco`. Whether that
//! graph happens to read the circuit's own state back (`meas:<node>`, making it what's
//! conventionally called "closed-loop") is just a property of how the blocks are wired, the
//! same as it would be in any real block-diagram simulation tool — the solver doesn't need to be told which case it
//! is, because it resolves both exactly the same way: **the error signal is a `sum` block's own
//! output, and a frequency-modulated PWM carrier is `sum -> pid -> sum -> vco`, not one fused
//! "closed-loop controller" that bakes a specific topology together.** Each block is declared
//! with `kind=<block>`, its own parameters, and `in=<signal>` (single-input blocks) or
//! `inputs=<signal>,<signal>,...` (`sum`, one per `signs=` entry). A `<signal>` is either
//! another block's name (its output this same step) or `meas:<node>` (the circuit's own
//! previous-step measurement, e.g. `meas:vout` for `V(vout)`). **Blocks are evaluated in the
//! order they appear in the file** — every signal must reference a block declared *earlier* (or
//! a `meas:` signal, which has no ordering constraint) — so declare sources first and sinks
//! last, same as you'd read a signal-flow diagram left to right.
//!
//! `kind=statespace a=<row>;<row>;... b=<v0,v1,...> c=<v0,v1,...> [d=<scalar>]` declares an
//! arbitrary single-input single-output block directly from its own matrices (`a`'s rows
//! semicolon-separated, each row comma-separated entries; `b`/`c` comma-separated vectors; `d`
//! defaults to `0`, the common case for a strictly-proper filter). `kind=tf num=<c0,c1,...>
//! den=<c0,c1,...>` declares one from a rational transfer function instead (coefficients
//! highest-degree first) — e.g. a PID's own realizable form `C(s) = Kp + Ki/s + Kd*N*s/(s+N)`
//! (a pure derivative term alone is non-causal, so every real PID filters it) put over one
//! denominator and given directly as `num`/`den`, instead of `kind=pid`'s `kp`/`ki`/`kd`/`n`
//! convenience parameterization.
//!
//! `kind=product inputs=<signal>,...` (multiplies all its inputs) and `kind=saturation
//! limit=<f64> in=<signal>` (clamps to `[-limit, limit]`) round out the stateless math ops.
//! `kind=table points=<x>:<y>,<x>:<y>,... in=<signal>` linearly interpolates through a fixed
//! lookup table (clamped, not extrapolated, past either end). `kind=hysteresis high=<f64>
//! low=<f64> in=<signal>` is a Schmitt-trigger comparator (output `1.0`/`0.0`): stays HIGH
//! until the input drops below `low`, stays LOW until it rises above `high` — the standard
//! bang-bang/hysteresis-band block for current-mode control, where there's no fixed switching
//! frequency to modulate a duty command onto. Beyond those, any `kind=` naming
//! a real-valued scalar function (`cos`, `sin`, `tan`, `exp`, `ln`, `log10`, `sqrt`, `abs`,
//! `sinh`/`cosh`/`tanh`, `asin`/`acos`/`atan`, `asinh`/`acosh`/`atanh`, `floor`/`ceil`/`round`/
//! `int`, `sgn`, `u`/`uramp` (unit step / ramp), `buf`/`inv` (threshold at 0.5) — each with
//! `in=<signal>`; `atan2`/`hypot`/`pow`/`pwr`/`pwrs`/`min`/`max` — each with `in1=`/`in2=`;
//! `if`/`limit` — each with `in1=`/`in2=`/`in3=`) resolves to that function as a block, no
//! separate `kind=` list to maintain — see `continuous_blocks::waveform_arithmetic` for the
//! full set and exactly what was left out (a derivative, noise/random generators, complex-data
//! functions, and Boolean/comparison operators — none of those are stateless real-valued math).
//!
//! `kind=cscript lib=<path> [in=<signal> | inputs=<signal>,...] [outputs=<name>,<name>,...]
//! [ts=<seconds> | freq=<hz>]` loads a user-supplied, precompiled shared library
//! (`.so`/`.dylib`/`.dll` — the user compiles it themselves, this CLI never invokes a compiler)
//! exporting `cscript_start`/`cscript_output`/(optionally) `cscript_free`/`cscript_clone` — an
//! escape hatch for block behavior none of `continuous-blocks`'s own blocks cover, including
//! genuinely stateful behavior (an integrator, a lookup table built at `cscript_start`,
//! anything). `outputs=` names more than one output signal from a single `cscript_output` call
//! (the block's own name aliases the *first* one); omit it for the common single-output case,
//! where the block's own name is the only output. `ts=`/`freq=` (mutually exclusive) give this
//! block its own fixed sample period, independent of the circuit's own resolved step size —
//! `cscript_output` only actually runs once accumulated time reaches `ts` (or `1/freq`), and
//! the block holds its last output (zero-order hold) on every step in between, the right model
//! for a genuinely discrete controller running at a fixed rate (e.g. a digital control loop
//! clocked well below the switching frequency) rather than something meant to behave
//! continuously; omit both to run `cscript_output` every resolved circuit step instead (the
//! right choice for a continuous-like block). **Adaptive step-size control (the default when
//! `--dt` is omitted) requires `cscript_clone` to be exported** — adaptive stepping clones
//! every block's state before each trial and discards it on a rejected trial, and an opaque C
//! state pointer can't be deep-copied without the library's own help; pass `--dt` (fixed-step)
//! instead if the library doesn't export it. See `cscript_ffi`'s own module doc comment for the
//! full C-side contract, why loading and calling into a shared library is unsafe by
//! construction, and what is and isn't checked.
//!
//! A MOSFET's gate can reference a controller block by name instead of a fixed/`pwm` spec, two
//! ways:
//! - `gate=vco ctrl=<vco-block-name> phase=<0..1> duty=<0..1>` — frequency modulation (LLC-
//!   family converters). Several gates naming the same `vco` share one oscillator with
//!   different phase offsets (a half-bridge's two complementary switches) rather than needing
//!   a separate oscillator per gate.
//! - `gate=dutyctrl ctrl=<block-name> freq=<hz>` — duty modulation at a fixed carrier frequency
//!   (buck/boost-style), with the duty *command* coming from anywhere in the graph instead of
//!   being a fixed value.
//! - `gate=block ctrl=<block-name>` — direct on/off control, on while the named block's output
//!   is `>= 0.5`, no carrier at all. Meant for a `kind=hysteresis` block (bang-bang current-mode
//!   control has no fixed switching frequency to compare against), but works with any block.
//!
//! Example — a frequency-modulated half-bridge PID (LLC-family converters regulate by
//! switching frequency, not PWM duty, unlike buck/boost) with a reference step test, as it
//! would appear inside the `.cir` file alongside the actual circuit elements (`V1`, `Lr`, ...):
//!
//! ```text
//! V1 vin 0 400
//! * D1 kind=mosfet r_on=0.01 g_breakdown=0 v_breakdown=-1e6 g_off=1e-6 v_th=1e6 g_on=0 gate=vco ctrl=VCO1 phase=0 duty=0.48
//! * D2 kind=mosfet r_on=0.01 g_breakdown=0 v_breakdown=-1e6 g_off=1e-6 v_th=1e6 g_on=0 gate=vco ctrl=VCO1 phase=0.5 duty=0.48
//! ... (Lr, Cr, transformer, rectifier, Cout, Rout -- ordinary SPICE elements)
//! * REF    kind=pwl points=0:20,0.014:17
//! * ERR    kind=sum inputs=REF,meas:vout signs=1,-1
//! * PID1   kind=pid kp=800 ki=4e6 kd=0 n=1000 clamp_lo=-15000 clamp_hi=15000 in=ERR
//! * FNOM   kind=const value=115000
//! * FREQ   kind=sum inputs=FNOM,PID1 signs=1,-1
//! * VCO1   kind=vco f_min=100000 f_max=130000 in=FREQ
//! ```
//!
//! A real SPICE tool opening this file sees seven ordinary comment lines and an otherwise
//! unremarkable LLC deck. `elspice-pwl-cli some.cir --mode transient` (no `--devices`) sees the
//! complete closed loop.
//!
//! `--mode dc` cannot resolve a block-driven gate (`gate=vco`/`gate=dutyctrl`/`gate=block`): a DC operating
//! point has no notion of the time-stepped state a `Pid`/`Vco` block carries, so those gate
//! kinds need `--mode transient`.
//!
//! See `internal-archive/experiments/elspice-pwl-llc-closed-loop-vs-xyce-ngspice/`
//! and `experiments/elspice-pwl-buck-underdamped-resonance-filter/` for full worked examples
//! this syntax was built for.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::process::ExitCode;

use continuous_blocks::{Hysteresis, Pid, StateSpace, TransferFunction, Vco};
use dae_runtime::{
    simulate_transient, simulate_transient_with_blocks, solve_dc, solve_dc_with_mosfets,
    AdaptiveConfig, BlockInstance, BlockKind, GateBinding, GateState, Signal, TimeStep,
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
    DutyCtrl { ctrl: String, freq_hz: f64 },
    Block { ctrl: String },
}

impl GateSpec {
    fn to_binding(&self) -> GateBinding {
        match self {
            GateSpec::Fixed(state) => GateBinding::Fixed(*state),
            GateSpec::Pwm { freq_hz, duty } => GateBinding::PwmFixed {
                freq_hz: *freq_hz,
                duty: *duty,
            },
            GateSpec::Vco { ctrl, phase, duty } => GateBinding::Vco {
                vco: ctrl.clone(),
                phase: *phase,
                duty: *duty,
            },
            GateSpec::DutyCtrl { ctrl, freq_hz } => GateBinding::Pwm {
                duty: ctrl.clone(),
                freq_hz: *freq_hz,
            },
            GateSpec::Block { ctrl } => GateBinding::Block(ctrl.clone()),
        }
    }
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
    let mut dt: Option<f64> = None;
    let mut dt_max: Option<f64> = None;
    let mut dt_min: Option<f64> = None;
    let mut dt_init: Option<f64> = None;
    let mut reltol: Option<f64> = None;
    let mut abstol: Option<f64> = None;

    let mut i = 2;
    while i < args.len() {
        let parse_f64 = |flag: &'static str, i: &mut usize| -> Result<f64, String> {
            let v = args
                .get(*i + 1)
                .ok_or_else(|| format!("{flag} needs a value"))?
                .parse()
                .map_err(|_| format!("bad {flag}"))?;
            *i += 2;
            Ok(v)
        };
        match args[i].as_str() {
            "--devices" => {
                devices_path = Some(args.get(i + 1).ok_or("--devices needs a value")?.clone());
                i += 2;
            }
            "--mode" => {
                mode = args.get(i + 1).ok_or("--mode needs a value")?.clone();
                i += 2;
            }
            "--tfinal" => t_final = parse_f64("--tfinal", &mut i)?,
            "--dt" => dt = Some(parse_f64("--dt", &mut i)?),
            "--dt-max" => dt_max = Some(parse_f64("--dt-max", &mut i)?),
            "--dt-min" => dt_min = Some(parse_f64("--dt-min", &mut i)?),
            "--dt-init" => dt_init = Some(parse_f64("--dt-init", &mut i)?),
            "--reltol" => reltol = Some(parse_f64("--reltol", &mut i)?),
            "--abstol" => abstol = Some(parse_f64("--abstol", &mut i)?),
            other => return Err(format!("unrecognized argument '{other}'\n{}", usage())),
        }
    }

    if dt.is_some()
        && (dt_max.is_some()
            || dt_min.is_some()
            || dt_init.is_some()
            || reltol.is_some()
            || abstol.is_some())
    {
        return Err(
            "--dt (fixed step) and --dt-max/--dt-min/--dt-init/--reltol/--abstol (adaptive \
             step) are mutually exclusive — pick one"
                .to_string(),
        );
    }
    // No --dt at all: adaptive, same as real SPICE tools auto-managing the internal step when
    // only the run length is given. `--dt-max`/`--dt-min`/`--dt-init` (and `--reltol`/
    // `--abstol`) override AdaptiveConfig::from_t_final's own defaults piecemeal.
    let step = match dt {
        Some(dt) => TimeStep::Fixed(dt),
        None => {
            let mut config = AdaptiveConfig::from_t_final(t_final);
            if let Some(v) = dt_max {
                config.dt_max = v;
            }
            if let Some(v) = dt_min {
                config.dt_min = v;
            }
            if let Some(v) = dt_init {
                config.dt_init = v;
            }
            if let Some(v) = reltol {
                config.reltol = v;
            }
            if let Some(v) = abstol {
                config.abstol = v;
            }
            TimeStep::Adaptive(config)
        }
    };

    let netlist =
        fs::read_to_string(netlist_path).map_err(|e| format!("reading {netlist_path}: {e}"))?;
    // No `--devices` given: look for device/block lines (marked `*...kind=...`, a plain SPICE
    // comment to every other tool) directly in the netlist file itself, so one `.cir` file can
    // be the complete, self-contained circuit — the same discipline a real SPICE deck already
    // has, rather than device/block declarations living in a second file split out by
    // convention alone. `--devices <file>` remains available for the (rarer) case of sharing
    // one controller/PWL-parameter file across several netlists.
    let devices = match &devices_path {
        Some(path) => {
            let text = fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
            parse_devices(&text)?
        }
        None => parse_devices(&netlist)?,
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
            let trace = simulate_transient(&netlist, dialect, &diodes, None, t_final, step)
                .map_err(|e| format!("{e:?}"))?;
            if let Some((_, first)) = trace.first() {
                print_header(&first.unknowns);
            }
            for (t, point) in &trace {
                print_row(*t, point);
            }
        } else {
            run_transient_with_mosfets(
                &netlist,
                dialect,
                &diodes,
                &mosfets,
                &blocks,
                shared_r_on,
                t_final,
                step,
            )?;
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
                GateSpec::Vco { .. } | GateSpec::DutyCtrl { .. } | GateSpec::Block { .. } => {
                    return Err(format!(
                        "device '{name}': gate=vco/dutyctrl/block needs --mode transient, not \
                         'dc' (a DC operating point has no notion of a block's time-stepped \
                         state)"
                    ))
                }
            };
            Ok((name.clone(), (*m, state)))
        })
        .collect()
}

/// `--mode transient` with at least one MOSFET: resolves every gate (`fixed`/`pwm`/`vco`/
/// `dutyctrl`, mixed freely) via [`dae_runtime::simulate_transient_with_blocks`] — see this
/// file's module doc comment for why there's no separate mode for the block-driven case.
#[allow(clippy::too_many_arguments)]
fn run_transient_with_mosfets(
    netlist: &str,
    dialect: Dialect,
    diodes: &BTreeMap<String, Diode>,
    mosfets: &BTreeMap<String, (Mosfet, GateSpec)>,
    blocks: &[BlockInstance],
    shared_r_on: f64,
    t_final: f64,
    step: TimeStep,
) -> Result<(), String> {
    let mosfets_only: BTreeMap<String, Mosfet> =
        mosfets.iter().map(|(n, (m, _))| (n.clone(), *m)).collect();
    let gates: BTreeMap<String, GateBinding> = mosfets
        .iter()
        .map(|(name, (_, gate))| (name.clone(), gate.to_binding()))
        .collect();

    let trace = simulate_transient_with_blocks(
        netlist,
        dialect,
        diodes,
        &mosfets_only,
        blocks,
        &gates,
        shared_r_on,
        None,
        t_final,
        step,
    )
    .map_err(|e| format!("{e:?}"))?;

    // A cscript block with `outputs=` naming more than one signal registers extra named
    // outputs beyond its own block name (see block_graph::evaluate_blocks) -- list those too,
    // so they show up as their own CSV columns instead of only being reachable via
    // Signal::Block from another declared block.
    let mut block_names: Vec<String> = blocks.iter().map(|b| b.name.clone()).collect();
    for block in blocks {
        if let BlockKind::CScript { output_names, .. } = &block.kind {
            block_names.extend(output_names.iter().skip(1).cloned());
        }
    }
    if let Some((_, first, _)) = trace.first() {
        if block_names.is_empty() {
            print_header(&first.unknowns);
        } else {
            println!("t,{},{}", first.unknowns.join(","), block_names.join(","));
        }
    }
    for (t, point, outputs) in &trace {
        if block_names.is_empty() {
            print_row(*t, point);
        } else {
            let values: Vec<String> = point.x.iter().map(|v| v.to_string()).collect();
            let block_values: Vec<String> = block_names
                .iter()
                .map(|name| outputs.get(name).copied().unwrap_or(f64::NAN).to_string())
                .collect();
            println!("{t},{},{}", values.join(","), block_values.join(","));
        }
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

/// Parses a `<x>:<y>,<x>:<y>,...` point list (a `kind=pwl` reference schedule, or a
/// `kind=table` lookup table), sorted ascending by `x` on return.
fn parse_xy_points(text: &str, name: &str, line_number: usize) -> Result<Vec<(f64, f64)>, String> {
    let mut points = Vec::new();
    for point in text.split(',') {
        let (x_str, y_str) = point.split_once(':').ok_or_else(|| {
            format!(
                "line {}: device '{name}' field 'points' entry '{point}' is not '<x>:<y>'",
                line_number + 1
            )
        })?;
        let x: f64 = x_str.parse().map_err(|_| {
            format!(
                "line {}: device '{name}' points entry '{point}': '{x_str}' is not a number",
                line_number + 1
            )
        })?;
        let y: f64 = y_str.parse().map_err(|_| {
            format!(
                "line {}: device '{name}' points entry '{point}': '{y_str}' is not a number",
                line_number + 1
            )
        })?;
        points.push((x, y));
    }
    points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    Ok(points)
}

/// Parses a comma-separated list of numbers, e.g. a `kind=statespace` block's `b`/`c` vector
/// or a `kind=tf` block's `num`/`den` coefficients.
fn parse_vector(
    text: &str,
    name: &str,
    field: &str,
    line_number: usize,
) -> Result<Vec<f64>, String> {
    text.split(',')
        .map(|v| {
            v.trim().parse::<f64>().map_err(|_| {
                format!(
                    "line {}: device '{name}' field '{field}' entry '{v}' is not a number",
                    line_number + 1
                )
            })
        })
        .collect()
}

/// Parses a `kind=statespace` block's `a` matrix: semicolon-separated rows, each a
/// comma-separated list of numbers.
fn parse_matrix_rows(
    text: &str,
    name: &str,
    field: &str,
    line_number: usize,
) -> Result<Vec<Vec<f64>>, String> {
    text.split(';')
        .map(|row| parse_vector(row, name, field, line_number))
        .collect()
}

fn parse_devices(text: &str) -> Result<Vec<(String, Kind)>, String> {
    let mut result = Vec::new();
    for (line_number, raw_line) in text.lines().enumerate() {
        let mut line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        // A device/block line embedded directly in a `.cir` file (so the same file is both
        // the netlist and the device/block source -- see this file's module doc comment)
        // needs a leading `*`, a plain SPICE comment marker, so spice-core's own parser skips
        // it as an ordinary comment. Strip that marker here before parsing.
        if let Some(rest) = line.strip_prefix('*') {
            line = rest.trim_start();
        }
        // Anything without `kind=` isn't a device/block declaration -- most likely a real
        // SPICE element line (when this same file is also the netlist) or a plain comment
        // that happens to start with `*`. Either way, not ours to parse.
        if !line.contains("kind=") {
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
                    Some("dutyctrl") => GateSpec::DutyCtrl {
                        ctrl: get_str("ctrl")?,
                        freq_hz: get("freq")?,
                    },
                    Some("block") => GateSpec::Block {
                        ctrl: get_str("ctrl")?,
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
                let points = parse_xy_points(&get_str("points")?, name, line_number)?;
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
            "hysteresis" => {
                let hysteresis = Hysteresis::new(get("high")?, get("low")?);
                Kind::Block(BlockInstance {
                    name: name.to_string(),
                    kind: BlockKind::Hysteresis(hysteresis),
                    inputs: vec![parse_signal(&get_str("in")?)],
                })
            }
            "cscript" => {
                let lib = std::path::PathBuf::from(get_str("lib")?);
                let output_names = match fields.get("outputs") {
                    Some(names) => names.split(',').map(str::to_string).collect(),
                    None => vec![name.to_string()],
                };
                let inputs = match fields.get("inputs") {
                    Some(list) => list.split(',').map(parse_signal).collect(),
                    None => vec![parse_signal(&get_str("in")?)],
                };
                let sample_time = match (fields.get("ts"), fields.get("freq")) {
                    (Some(_), Some(_)) => {
                        return Err(format!(
                            "line {}: device '{name}': 'ts' and 'freq' are mutually exclusive \
                             (both set this block's sample time)",
                            line_number + 1
                        ))
                    }
                    (Some(_), None) => Some(get("ts")?),
                    (None, Some(_)) => Some(1.0 / get("freq")?),
                    (None, None) => None,
                };
                Kind::Block(BlockInstance {
                    name: name.to_string(),
                    kind: BlockKind::CScript {
                        lib,
                        output_names,
                        sample_time,
                    },
                    inputs,
                })
            }
            "statespace" => {
                let a = parse_matrix_rows(&get_str("a")?, name, "a", line_number)?;
                let b_vec = parse_vector(&get_str("b")?, name, "b", line_number)?;
                let c_vec = parse_vector(&get_str("c")?, name, "c", line_number)?;
                let d = fields
                    .get("d")
                    .map(|s| {
                        s.parse::<f64>().map_err(|_| {
                            format!(
                                "line {}: device '{name}' field 'd' is not a number",
                                line_number + 1
                            )
                        })
                    })
                    .transpose()?
                    .unwrap_or(0.0);
                let b: Vec<Vec<f64>> = b_vec.into_iter().map(|v| vec![v]).collect();
                let c: Vec<Vec<f64>> = vec![c_vec];
                let ss = StateSpace {
                    a,
                    b,
                    c,
                    d: vec![vec![d]],
                    e: None,
                };
                Kind::Block(BlockInstance {
                    name: name.to_string(),
                    kind: BlockKind::StateSpace(ss),
                    inputs: vec![parse_signal(&get_str("in")?)],
                })
            }
            "tf" => {
                let num = parse_vector(&get_str("num")?, name, "num", line_number)?;
                let den = parse_vector(&get_str("den")?, name, "den", line_number)?;
                let tf = TransferFunction::new(num, den).map_err(|e| {
                    format!(
                        "line {}: device '{name}': invalid transfer function ({e:?})",
                        line_number + 1
                    )
                })?;
                Kind::Block(BlockInstance {
                    name: name.to_string(),
                    kind: BlockKind::TransferFunction(tf),
                    inputs: vec![parse_signal(&get_str("in")?)],
                })
            }
            "product" => Kind::Block(BlockInstance {
                name: name.to_string(),
                kind: BlockKind::Product,
                inputs: get_str("inputs")?.split(',').map(parse_signal).collect(),
            }),
            "saturation" => Kind::Block(BlockInstance {
                name: name.to_string(),
                kind: BlockKind::Saturation(get("limit")?),
                inputs: vec![parse_signal(&get_str("in")?)],
            }),
            "table" => {
                let points = parse_xy_points(&get_str("points")?, name, line_number)?;
                Kind::Block(BlockInstance {
                    name: name.to_string(),
                    kind: BlockKind::Table(points),
                    inputs: vec![parse_signal(&get_str("in")?)],
                })
            }
            other => {
                // Not one of the block kinds above: try the real-valued waveform-arithmetic
                // function library (cos, sin, exp, sqrt, atan2, hypot, if, limit, ...) before
                // giving up — see `continuous_blocks::waveform_arithmetic` for the full list.
                if let Some(f) = continuous_blocks::MathFn1::from_name(other) {
                    Kind::Block(BlockInstance {
                        name: name.to_string(),
                        kind: BlockKind::MathFn1(f),
                        inputs: vec![parse_signal(&get_str("in")?)],
                    })
                } else if let Some(f) = continuous_blocks::MathFn2::from_name(other) {
                    Kind::Block(BlockInstance {
                        name: name.to_string(),
                        kind: BlockKind::MathFn2(f),
                        inputs: vec![
                            parse_signal(&get_str("in1")?),
                            parse_signal(&get_str("in2")?),
                        ],
                    })
                } else if let Some(f) = continuous_blocks::MathFn3::from_name(other) {
                    Kind::Block(BlockInstance {
                        name: name.to_string(),
                        kind: BlockKind::MathFn3(f),
                        inputs: vec![
                            parse_signal(&get_str("in1")?),
                            parse_signal(&get_str("in2")?),
                            parse_signal(&get_str("in3")?),
                        ],
                    })
                } else {
                    return Err(format!(
                        "line {}: unknown device kind '{other}'",
                        line_number + 1
                    ));
                }
            }
        };
        result.push((name.to_string(), entry));
    }
    Ok(result)
}

fn usage() -> String {
    "usage: elspice-pwl <netlist> [--devices <file>] [--mode dc|transient] [--tfinal T] \
     [--dt DT | --dt-max T --dt-min T --dt-init T --reltol R --abstol A]\n\
     \n\
     --dt fixes the step size every step (deterministic, exactly reproducible). Omit it (and \
     optionally tune --dt-max/--dt-min/--dt-init/--reltol/--abstol) for adaptive step-size \
     control instead — small steps where the solution is changing fast, large steps where it's \
     settled, the same local-truncation-error approach every real SPICE-family tool uses by \
     default. --dt and any --dt-*/--reltol/--abstol flag are mutually exclusive."
        .to_string()
}
