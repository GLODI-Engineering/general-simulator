//! Thin runner for `general-simulator`: `general-simulator <netlist> [--devices <file>] --mode
//! {dc|transient} [--tfinal T] [--dt DT | --dt-max/--dt-min/--dt-init/--reltol/--abstol]`
//! prints a CSV waveform (`t,V(node1),V(node2),...`) to stdout, one row per resolved timestep
//! (a single row for `--mode dc`). `--dt` fixes the step size; omit it for adaptive step-size
//! control instead (the default — see `dae_runtime::TimeStep`/`AdaptiveConfig`'s own doc
//! comments for the algorithm), with `--dt-max`/`--dt-min`/`--dt-init`/`--reltol`/`--abstol`
//! overriding individual defaults derived from `--tfinal`.
//!
//! **The netlist is the one file** — PWL device parameters and any controller block graph
//! live directly inside it, the same way a real SPICE deck is self-contained, not split across
//! files by convention. **This binary no longer parses any of that itself** — `general-mna`'s
//! `build_system` does, via `general-spice-core`'s real grammar (see its own
//! `docs/GRAMMAR.md` §12), which now has a genuine first-class syntax for a `kind=mosfet
//! r_on=...`/block-graph line, no `*`-comment disguise needed:
//!
//! ```text
//! D1 kind=diode g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.7 g_on=1
//! ONVAL kind=const value=1
//! ONGATE kind=sig2gate in=ONVAL
//! D2 kind=mosfet r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.5 g_on=5 gate=block ctrl=ONGATE
//! DUTY kind=const value=0.6
//! PWM1 kind=pwm freq=10000 in=DUTY outputs=PWM1_ON,PWM1_OFF
//! PWM1G kind=sig2gate in=PWM1_ON
//! D3 kind=mosfet r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.5 g_on=5 gate=block ctrl=PWM1G
//! ```
//!
//! The older convention — the same lines, each prefixed with `*` so a real SPICE tool (or an
//! un-migrated netlist) sees an ordinary comment — still works unchanged; `general-spice-core`'s
//! lexer still strips `*`-prefixed lines to nothing before parsing either way, so a mix of old-
//! and new-style lines in the same file is fine. This is the whole netlist: run
//! `general-simulator some.cir --mode transient` with no `--devices` at all and `build_system`
//! looks for these lines in `some.cir` itself. `--devices <file>` remains available for the
//! rarer case of sharing one controller/PWL-parameter file across several netlists, but is not
//! the default or the expected common case.
//!
//! Every MOSFET must declare the same `r_on` — `dae-runtime`'s switch mechanism uses one shared
//! on-resistance per call (see `dae_runtime::solve_dc_with_mosfets`'s doc comment).
//!
//! ## Wiring a controller into a gate
//!
//! **There is no separate "closed-loop mode," and no non-block-driven gate at all.** Every
//! MOSFET's gate is `gate=block ctrl=<sig2gate-name>` — always resolved the same way, every
//! step, by reading the named block's current output (`>= 0.5` means on). Even a permanently-on
//! or permanently-off gate is an ordinary block (`kind=const value=1`, wrapped in a
//! `kind=sig2gate` like any other gate target), not a special fixed-state escape hatch — see
//! "Physical/signal-domain converters" below for why every `ctrl=` target must specifically be a
//! `sig2gate` converter. `continuous-blocks` supplies the vocabulary the device file wires
//! together — `const`, `time` (zero-input, outputs the current step's own simulated time — the
//! standard "clock" source, needed to build a `sin(2*pi*f*t)`-style signal via `MathFn1`/`Gain`
//! since no block otherwise sees `t` directly), `pwc`/`pwl` (piecewise-constant/-linear
//! sources, e.g. a reference schedule), `sum` (an error junction with explicit `+`/`-` signs),
//! `gain`, `pid`, `statespace` (arbitrary `(A,B,C,D)`), `tf` (a rational `N(s)/D(s)`), `vco` (a
//! bare oscillator ramp), and the two PWM modulators this vocabulary is ultimately *for* —
//! `pwm`/`pspwm`, see their own section below. Whether a graph happens to read the circuit's own
//! state back (through a `kind=probe`, see below,
//! making it what's conventionally called "closed-loop") is just a property of how the blocks
//! are wired, the same as it would be in any real block-diagram simulation tool — the solver
//! doesn't need to be told which case it is, because it resolves both exactly the same way:
//! **the error signal is a `sum` block's own output, and a frequency-modulated PWM carrier is
//! `sum -> pid -> sum -> pspwm` (PWM Modulator 2), not one fused "closed-loop controller" that
//! bakes a specific topology together.** Each block is declared with `kind=<block>`, its own
//! parameters, and
//! `in=<signal>` (single-input blocks) or `inputs=<signal>,<signal>,...` (`sum`, one per
//! `signs=` entry). A `<signal>` is another block's name (its output this same step) or
//! `prev:<block>` (that — or any — named block's own output from the *previous* step, `0.0`
//! before the first step): needed to close a loop *around a block itself* rather than around
//! the circuit — a current controller regulating a `kind=pmsm`'s own `id`/`iq` outputs, or a
//! PLL's angle estimate feeding the very `kind=park` block that produced its own error signal,
//! would otherwise be a same-step algebraic loop; `prev:` is the one-sample delay every real
//! digital controller reading its own last output already has. **Blocks are *not* evaluated in
//! file order** — each step's actual evaluation order is derived automatically from the
//! `in=`/`inputs=` dependency graph itself (`dae_runtime::block_graph::topological_order`), so
//! a block may name another declared anywhere in the file, before or after it; only `prev:` was
//! ever exempt from an ordering rule before, and now nothing needs one, since there's no longer
//! a file-position rule to be exempt from. A genuine same-step cycle among `in=`/`inputs=`
//! references (however indirect, including a block naming itself) is rejected before any step
//! runs, reported as the exact path that closes it (`dae_runtime::DaeError::AlgebraicLoop`) —
//! fix it by routing one edge in the cycle through `prev:` instead, the sanctioned way to turn a
//! same-step loop into a legitimate one-sample-delayed feedback path. Declaring sources before
//! sinks, left-to-right like a signal-flow diagram, remains good practice for a human reading
//! the file — it's just no longer a correctness requirement.
//!
//! ## Physical/signal-domain converters (enforced)
//!
//! A circuit quantity (a node voltage, a branch current) and a signal-domain block's output are
//! **not the same kind of thing** and cannot be wired together directly — the same rule
//! a reference tool/Simscape enforces with its own PS-a reference tool Converter / a reference tool-PS Converter
//! blocks, applied here at the netlist level (a companion UI is intended to enforce the same
//! rule visually later; this grammar is the ground truth). There are three converters, one per
//! crossing:
//!
//! - `kind=probe node=<name>` (reads `V(node)`) or `kind=probe branch=<name>` (reads
//!   `I(branch)`, mutually exclusive with `node=`) is the **only** way a circuit quantity enters
//!   the signal domain. Zero inputs (a source block, like `const`/`time`); reference its output
//!   afterward exactly like any other block's, e.g. `ERR kind=sum inputs=REF,VOUT_PROBE
//!   signs=1,-1` where `VOUT_PROBE kind=probe node=vout` was declared earlier.
//! - `kind=sig2gate in=<signal>` is the **only** legal target for a `gate=block ctrl=` field —
//!   the named block must be a `sig2gate` converter, not a raw `pid`/`vco`/`pwm`/`hysteresis`/
//!   etc. block directly (`dae_runtime::DaeError::GateTargetNotSig2Gate` otherwise). Purely an
//!   identity pass-through numerically — its whole purpose is marking, in the netlist text,
//!   exactly where a signal stops being "a number a controller computed" and starts being "a
//!   command that actuates a physical switch."
//! - `kind=sig2voltage in=<signal>` / `kind=sig2current in=<signal>` are the **only** legal way a
//!   signal-domain block drives an independent voltage/current source's own magnitude — name
//!   the converter block directly as that source's own literal value in the netlist, e.g. `V1 a
//!   0 VDRV` where `VDRV kind=sig2voltage in=CTRL` was declared earlier (`general-mna` already
//!   accepts a bare symbol there; no change was needed on that side). `V` sources need
//!   `sig2voltage`, `I` sources need `sig2current` — a mismatch, or naming any other kind of
//!   block, is rejected
//!   (`dae_runtime::DaeError::SourceNotSig2PhysicalConverter`) before any step runs. This closes
//!   a real, previously-open gap: without it, a block could only ever *observe* the circuit
//!   (via `kind=probe`), never load or drive it — see `elspice-pwl-buck-dc-motor-cascade` in the
//!   sibling `internal-archive` repo for the concrete limitation this fixes.
//!
//! `kind=pid kp=<f64> ki=<f64> kd=<f64> n=<f64> in=<signal>` plus either `clamp_lo=<f64>
//! clamp_hi=<f64>` (a fixed anti-windup bound, the common case) or `clamp_lo_in=<signal>
//! clamp_hi_in=<signal>` (a *dynamic* bound, read fresh every step like any other input,
//! exactly mutually exclusive with the fixed form) declares a PID with two-sided
//! conditional-integration anti-windup. The dynamic form exists for a controller whose
//! achievable output range genuinely depends on other, still-evolving state — e.g. a
//! current-loop PID commanding a pole voltage that can't physically exceed roughly half the DC
//! bus voltage, itself still rising during a soft-start ramp: a *fixed* bound sized for the
//! final steady-state range is badly oversized early on, so the PID's own anti-windup never
//! actually engages even though the real plant is already saturated far below that fixed
//! bound — a real, previously hard-to-diagnose failure mode this dynamic form fixes directly
//! (see `dae_runtime::PidClamp`'s own doc comment for the worked example this was built for).
//!
//! `kind=statespace a=[[...],...] b=[...] c=[...] [d=<scalar>]` declares an arbitrary
//! single-input single-output block directly from its own matrices — every list-valued field in
//! this grammar (`a`/`b`/`c` here, `num`/`den` below, `points` on `pwc`/`pwl`/`table`) is a real
//! Python list literal (`a=[[1,2],[3,4]]` is exactly `numpy.array([[1,2],[3,4]])`'s own shape;
//! `b=[1,2]` a plain list), so a matrix/vector built in Python can be pasted in unchanged. `d`
//! defaults to `0`, the common case for a strictly-proper filter. `kind=tf num=[c0,c1,...]
//! den=[c0,c1,...]` declares one from a rational transfer function instead (coefficients
//! highest-degree first) — e.g. a PID's own realizable form `C(s) = Kp + Ki/s + Kd*N*s/(s+N)`
//! (a pure derivative term alone is non-causal, so every real PID filters it) put over one
//! denominator and given directly as `num`/`den`, instead of `kind=pid`'s `kp`/`ki`/`kd`/`n`
//! convenience parameterization.
//!
//! `kind=product inputs=<signal>,...` (multiplies all its inputs) and `kind=saturation
//! limit=<f64> in=<signal>` (clamps to `[-limit, limit]`) round out the stateless math ops.
//! `kind=table points=[[x0,y0],[x1,y1],...] in=<signal>` linearly interpolates through a fixed
//! lookup table (clamped, not extrapolated, past either end). `kind=hysteresis high=<f64>
//! low=<f64> in=<signal>` is a Schmitt-trigger comparator (output `1.0`/`0.0`): stays HIGH
//! until the input drops below `low`, stays LOW until it rises above `high` — the standard
//! bang-bang/hysteresis-band block for current-mode control, where there's no fixed switching
//! frequency to modulate a duty command onto. Beyond those, any `kind=` naming
//! a real-valued scalar function (`cos`, `sin`, `tan`, `exp`, `ln`, `log10`, `sqrt`, `abs`,
//! `sinh`/`cosh`/`tanh`, `asin`/`acos`/`atan`, `asinh`/`acosh`/`atanh`, `floor`/`ceil`/`round`/
//! `int`, `sgn`, `u`/`uramp` (unit step / ramp), `buf`/`inv` (threshold at 0.5) — each with
//! `in=<signal>`; `atan2`/`anglewrap`/`hypot`/`pow`/`pwr`/`pwrs`/`min`/`max` — each with
//! `in1=`/`in2=` (`anglewrap(alpha, beta)` is `atan2` wrapped to `[0, 2*pi)` — the
//! angle-tracking half of a synchronous-reference-frame PLL, feeding a `kind=park`/
//! `kind=clarkepark` block's `theta` input);
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
//! `kind=clarke inputs=<a>,<b>,<c>` / `kind=clarkeinv inputs=<alpha>,<beta>,<zero>` / `kind=park
//! inputs=<alpha>,<beta>,<zero>,<theta>` / `kind=parkinv inputs=<d>,<q>,<zero>,<theta>` /
//! `kind=clarkepark inputs=<a>,<b>,<c>,<theta>` / `kind=clarkeparkinv
//! inputs=<d>,<q>,<zero>,<theta>` are the six Clarke/Park coordinate transforms (see
//! `continuous_blocks::coordinate_transforms`) — the standard change of basis between a
//! three-phase `abc` quantity, its stationary `alpha`/`beta`/zero-sequence projection, and a
//! `d`/`q`/zero-sequence frame rotating at a given angle `theta` (radians), used to regulate a
//! three-phase grid or motor-drive quantity with an ordinary `pid` on a DC-like `d`/`q` value
//! instead of chasing a sine wave directly. Each is multi-output (3 outputs, same convention as
//! `kind=cscript`'s `outputs=`): the block's own name aliases the first (primary) output,
//! `outputs=<name>,<name>,<name>` names all three explicitly, or omit it entirely for
//! auto-generated names built from this transform's own conventional output names (e.g.
//! `kind=clarke` without `outputs=` on a block named `PLL` gives `PLL` (alpha), `PLL_beta`,
//! `PLL_zero`).
//!
//! `kind=pmsm r_s=<ohm> l_d=<H> l_q=<H> lambda_pm=<Wb> pole_pairs=<n> inertia=<kg*m^2>
//! friction=<N*m*s/rad> inputs=<vd>,<vq>,<t_load>` is a permanent-magnet synchronous motor in
//! the rotor `d`/`q` frame (see `continuous_blocks::Pmsm`) — genuinely nonlinear (bilinear
//! speed/current coupling), integrated with its own RK4 stepper rather than compiled to a
//! `statespace`. Four outputs, same `outputs=`/default-naming convention as `kind=clarke`
//! above: `id`, `iq` (A), `omega_m` (mechanical speed, rad/s), `theta_e` (electrical angle,
//! already wrapped to `[0, 2*pi)` — feed directly into a `kind=park`/`kind=clarkepark` block's
//! `theta` input). Starts at rest (`id=iq=omega_m=theta_e=0`).
//!
//! ## The two PWM modulators
//!
//! Every gate-driving PWM waveform in this crate is built from one of exactly two block kinds
//! — both **active-high complementary** (two outputs, `main` and `complement`, `complement`
//! being the exact logical NOT of `main`, never an inverted-logic-level signal) with
//! independent per-edge dead time, sharing one implementation
//! (`continuous_blocks::math_ops::complementary_pwm_with_deadtime`) so "how dead time is
//! inserted" has exactly one answer regardless of which modulator produced the pair. Dead time
//! delays only the two *rising* (turn-on) edges — `red` delays `main`'s own turn-on, `fed`
//! delays `complement`'s own turn-on — never a falling edge, which is what guarantees both
//! outputs are provably low during the gap (whichever switch was conducting always turns off
//! exactly on schedule; only the *other* one is held off a little longer before it's allowed to
//! turn on). `red=fed=0.0` (the default for both) recovers the ideal, gap-free, overlap-free
//! pair exactly.
//!
//! - **PWM Modulator 1** — `kind=pwm freq=<hz> in=<duty-signal> [red=<seconds>] [fed=<seconds>]
//!   [outputs=<main>,<complement>]` — fixed carrier frequency, block-driven duty (the standard
//!   buck/boost-style comparator). `outputs=` defaults to `<name>,<name>_comp`.
//! - **PWM Modulator 2** — `kind=pspwm f_min=<hz> f_max=<hz>
//!   inputs=<freq-signal>,<phase-signal>,<duty-signal> [red=<seconds>] [fed=<seconds>]
//!   [outputs=<main>,<complement>]` — frequency, phase, *and* duty all block-driven
//!   ("phase-shift PWM," the standard term for exactly the modulation scheme a
//!   dual-active-bridge/phase-shifted-full-bridge converter uses). Owns its own frequency-
//!   integration state directly (not a variant of `kind=vco`, though it reuses the same
//!   clamp-and-integrate math internally) — two `pspwm` instances fed the *same* `freq` input
//!   stay phase-synchronized (deterministic integration, same `dt`, same starting phase `0.0`),
//!   the way a bridge's two legs need to be, without a separately-declared shared oscillator
//!   block in between. `red`/`fed` are converted to a phase fraction using *this step's own*
//!   resolved frequency (not a fixed constant), since the same absolute dead time eats a larger
//!   fraction of the period at higher switching frequency — a real effect on a
//!   variable-frequency converter's own ZVS margin, not just bookkeeping. `outputs=` defaults
//!   the same way as PWM Modulator 1.
//!
//! Both feed `gate=block ctrl=<sig2gate-name>` on each switch — one `sig2gate` wrapping
//! `main`, another wrapping `complement`, for a true half-bridge leg's two switches; a topology
//! with only one actively-driven switch (a buck's own high-side, freewheeling through a diode)
//! just leaves the `complement` output unwired. **Every `ctrl=` target must resolve to a
//! `kind=sig2gate` converter** (see "Physical/signal-domain converters" above), never the raw
//! `pwm`/`pspwm`/`hysteresis`/etc. block directly.
//!
//! Example — a frequency-modulated half-bridge PID (LLC-family converters regulate by
//! switching frequency, not PWM duty, unlike buck/boost) with a reference step test, as it
//! would appear inside the `.cir` file alongside the actual circuit elements (`V1`, `Lr`, ...):
//!
//! ```text
//! V1 vin 0 400
//! * D1 kind=mosfet r_on=0.01 g_breakdown=0 v_breakdown=-1e6 g_off=1e-6 v_th=1e6 g_on=0 gate=block ctrl=LEG_MAIN_G
//! * D2 kind=mosfet r_on=0.01 g_breakdown=0 v_breakdown=-1e6 g_off=1e-6 v_th=1e6 g_on=0 gate=block ctrl=LEG_COMP_G
//! ... (Lr, Cr, transformer, rectifier, Cout, Rout -- ordinary SPICE elements)
//! * VOUT_PROBE kind=probe node=vout
//! * REF     kind=pwc points=[[0,20],[0.014,17]]
//! * ERR     kind=sum inputs=REF,VOUT_PROBE signs=1,-1
//! * PID1    kind=pid kp=800 ki=4e6 kd=0 n=1000 clamp_lo=-15000 clamp_hi=15000 in=ERR
//! * FNOM    kind=const value=115000
//! * FREQ    kind=sum inputs=FNOM,PID1 signs=1,-1
//! * ZEROPH  kind=const value=0
//! * HALFDUTY kind=const value=0.48
//! * LEG     kind=pspwm f_min=100000 f_max=130000 inputs=FREQ,ZEROPH,HALFDUTY outputs=LEG_MAIN,LEG_COMP
//! * LEG_MAIN_G kind=sig2gate in=LEG_MAIN
//! * LEG_COMP_G kind=sig2gate in=LEG_COMP
//! ```
//!
//! A real SPICE tool opening this file sees eleven ordinary comment lines and an otherwise
//! unremarkable LLC deck. `general-simulator-cli some.cir --mode transient` (no `--devices`) sees the
//! complete closed loop.
//!
//! `--mode dc` cannot resolve any gate at all now that every gate is block-driven: a DC
//! operating point has no notion of the time-stepped state a `Pid`/`Vco`/`Pwm`/`PhaseShiftPwm`
//! block carries, so any netlist with a MOSFET needs `--mode transient`.
//!
//! See `internal-archive/experiments/elspice-pwl-llc-closed-loop-vs-xyce-ngspice/`
//! and `experiments/elspice-pwl-buck-underdamped-resonance-filter/` for full worked examples
//! this syntax was built for.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::process::ExitCode;

use dae_runtime::{
    simulate_transient, simulate_transient_with_blocks, solve_dc, AdaptiveConfig, BlockInstance,
    BlockKind, GateBinding, TimeStep,
};
use general_spice_core::Dialect;
use pwl_devices::{Diode, Mosfet};

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
    let dialect = Dialect::Ngspice;
    // No `--devices` given: look for device/block lines directly in the netlist file itself, so
    // one `.cir` file can be the complete, self-contained circuit — the same discipline a real
    // SPICE deck already has, rather than device/block declarations living in a second file
    // split out by convention alone. `--devices <file>` remains available for the (rarer) case
    // of sharing one controller/PWL-parameter file across several netlists. This binary parses
    // none of it itself — `general_mna::build_system` does, via `general-spice-core`'s real
    // grammar (see this file's own module doc comment).
    let devices_source = match &devices_path {
        Some(path) => fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?,
        None => netlist.clone(),
    };
    let general_mna::System {
        diodes,
        mosfets,
        gates,
        blocks,
        shared_r_on,
        ..
    } = general_mna::build_system(&devices_source, dialect)
        .map_err(|e| format!("parsing device/block declarations: {e}"))?;

    if mode == "dc" {
        let point = if mosfets.is_empty() {
            solve_dc(&netlist, dialect, &diodes).map_err(|e| format!("{e:?}"))?
        } else {
            // Every gate is block-driven (gate=block) -- a DC operating point has no notion of
            // a block's time-stepped state, so any MOSFET at all makes --mode dc unsupported.
            let name = mosfets.keys().next().expect("mosfets is non-empty here");
            return Err(format!(
                "device '{name}': every gate is block-driven (gate=block), which needs \
                 --mode transient, not 'dc' (a DC operating point has no notion of a block's \
                 time-stepped state)"
            ));
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
                &gates,
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

/// `--mode dc` has no notion of a block's time-stepped state (no transient loop runs at all),
/// but every gate is now block-driven (`gate=block ctrl=<name>`, enforced by `general_mna::
/// build_system` itself) — so a `.op`-style DC operating point with any MOSFET present has
/// nothing to resolve its gate from and is unconditionally unsupported, not just for the cases
/// that used to need a block. `--mode transient` with at least one MOSFET: resolves every gate
/// (always block-driven) via [`dae_runtime::simulate_transient_with_blocks`] — see this file's
/// module doc comment for why there's no separate mode for the block-driven case.
#[allow(clippy::too_many_arguments)]
fn run_transient_with_mosfets(
    netlist: &str,
    dialect: Dialect,
    diodes: &BTreeMap<String, Diode>,
    mosfets: &BTreeMap<String, Mosfet>,
    gates: &BTreeMap<String, GateBinding>,
    blocks: &[BlockInstance],
    shared_r_on: f64,
    t_final: f64,
    step: TimeStep,
) -> Result<(), String> {
    let trace = simulate_transient_with_blocks(
        netlist,
        dialect,
        diodes,
        mosfets,
        blocks,
        gates,
        shared_r_on,
        None,
        t_final,
        step,
    )
    .map_err(|e| format!("{e:?}"))?;

    // A cscript, coordinate-transform, pmsm, pwm, or pspwm block registers extra named outputs
    // beyond its own block name (see block_graph::evaluate_blocks) -- list those too, so they
    // show up as their own CSV columns instead of only being reachable via Signal::Block from
    // another declared block.
    let mut block_names: Vec<String> = blocks.iter().map(|b| b.name.clone()).collect();
    for block in blocks {
        match &block.kind {
            BlockKind::CScript { output_names, .. }
            | BlockKind::PyBlock { output_names, .. }
            | BlockKind::CoordinateTransform { output_names, .. }
            | BlockKind::Pmsm { output_names, .. }
            | BlockKind::Pwm { output_names, .. }
            | BlockKind::PhaseShiftPwm { output_names, .. } => {
                block_names.extend(output_names.iter().skip(1).cloned());
            }
            _ => {}
        }
    }
    // A name whose value is a SignalValue::Vector expands into one CSV column per element
    // (NAME[0], NAME[1], ...) rather than one column holding the whole vector -- arity is fixed
    // for the whole run once a block declares it (see book/dev-guide/src/vector-signals.md), so
    // deriving each name's own column count from the *first* step's resolved shape is exactly
    // as valid as a separate static analysis would be, and reuses this function's own existing
    // "build the header from trace.first()" convention rather than a new mechanism.
    if let Some((_, first, first_outputs)) = trace.first() {
        if block_names.is_empty() {
            print_header(&first.unknowns);
        } else {
            let headers: Vec<String> = block_names
                .iter()
                .flat_map(|name| match first_outputs.get(name) {
                    Some(dae_runtime::SignalValue::Vector(v)) => (0..v.len())
                        .map(|i| format!("{name}[{i}]"))
                        .collect::<Vec<_>>(),
                    _ => vec![name.clone()],
                })
                .collect();
            println!("t,{},{}", first.unknowns.join(","), headers.join(","));
        }
    }
    for (t, point, outputs) in &trace {
        if block_names.is_empty() {
            print_row(*t, point);
        } else {
            let values: Vec<String> = point.x.iter().map(|v| v.to_string()).collect();
            let block_values: Vec<String> = block_names
                .iter()
                .flat_map(|name| match outputs.get(name) {
                    Some(dae_runtime::SignalValue::Scalar(x)) => vec![x.to_string()],
                    Some(dae_runtime::SignalValue::Vector(v)) => {
                        v.iter().map(|x| x.to_string()).collect()
                    }
                    None => vec![f64::NAN.to_string()],
                })
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

/// Parses a `<signal>` field value: `prev:<block>` for a named block's own output from the
/// *previous* step (`0.0` before the first step) — needed to close a loop around a block itself
/// (a controller regulating a `kind=pmsm`'s own `id`/`iq` outputs, or a PLL's angle estimate
/// feeding the very `kind=park` block that produced its own error signal), where a same-step
/// reference would be a genuine algebraic loop. Anything else is another block's name (this
/// same step's output) — including a `kind=probe` block, the *only* legal way to read a circuit
/// quantity into the signal domain (there is deliberately no `meas:`-style inline shortcut
/// anymore; see `kind=probe`'s own doc section above). A stray `meas:<node>` left over from
/// before this convention was enforced is simply treated as an ordinary (and therefore unknown)
/// block name, surfacing as a clear `UnknownBlockInput` error rather than silently reading the
/// circuit.
fn usage() -> String {
    "usage: general-simulator <netlist> [--devices <file>] [--mode dc|transient] [--tfinal T] \
     [--dt DT | --dt-max T --dt-min T --dt-init T --reltol R --abstol A]\n\
     \n\
     --dt fixes the step size every step (deterministic, exactly reproducible). Omit it (and \
     optionally tune --dt-max/--dt-min/--dt-init/--reltol/--abstol) for adaptive step-size \
     control instead — small steps where the solution is changing fast, large steps where it's \
     settled, the same local-truncation-error approach every real SPICE-family tool uses by \
     default. --dt and any --dt-*/--reltol/--abstol flag are mutually exclusive."
        .to_string()
}
