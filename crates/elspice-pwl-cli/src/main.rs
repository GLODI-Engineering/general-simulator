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
//! * ONVAL kind=const value=1
//! * ONGATE kind=sig2gate in=ONVAL
//! * D2 kind=mosfet r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.5 g_on=5 gate=block ctrl=ONGATE
//! * DUTY kind=const value=0.6
//! * PWM1 kind=pwm freq=10000 in=DUTY outputs=PWM1_ON,PWM1_OFF
//! * PWM1G kind=sig2gate in=PWM1_ON
//! * D3 kind=mosfet r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.5 g_on=5 gate=block ctrl=PWM1G
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
//!   0 VDRV` where `VDRV kind=sig2voltage in=CTRL` was declared earlier (`elspice-mna` already
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
//! unremarkable LLC deck. `elspice-pwl-cli some.cir --mode transient` (no `--devices`) sees the
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

use continuous_blocks::{CoordinateTransform, Hysteresis, Pid, StateSpace, TransferFunction, Vco};
use dae_runtime::{
    simulate_transient, simulate_transient_with_blocks, solve_dc, AdaptiveConfig, BlockInstance,
    BlockKind, GateBinding, PidClamp, ProbeTarget, Signal, TimeStep, TransientFunction,
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

/// Every gate is block-driven — see `dae_runtime::GateBinding`'s own doc comment for why there
/// is no non-block-driven variant left (a permanently-off gate is an explicit `Const(0.0)`
/// wired through `kind=sig2gate`, the same as any other gate).
#[derive(Clone)]
struct GateSpec {
    ctrl: String,
}

impl GateSpec {
    fn to_binding(&self) -> GateBinding {
        GateBinding::Block(self.ctrl.clone())
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
/// but every gate is now block-driven (`GateSpec` is always `gate=block ctrl=<name>`) — so a
/// `.op`-style DC operating point with any MOSFET present has nothing to resolve its gate from
/// and is unconditionally unsupported, not just for the cases that used to need a block.
/// `--mode transient` with at least one MOSFET: resolves every gate (always block-driven,
/// `gate=block`) via [`dae_runtime::simulate_transient_with_blocks`] — see this file's module
/// doc comment for why there's no separate mode for the block-driven case.
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

    // A cscript, coordinate-transform, pmsm, pwm, or pspwm block registers extra named outputs
    // beyond its own block name (see block_graph::evaluate_blocks) -- list those too, so they
    // show up as their own CSV columns instead of only being reachable via Signal::Block from
    // another declared block.
    let mut block_names: Vec<String> = blocks.iter().map(|b| b.name.clone()).collect();
    for block in blocks {
        match &block.kind {
            BlockKind::CScript { output_names, .. }
            | BlockKind::CoordinateTransform { output_names, .. }
            | BlockKind::Pmsm { output_names, .. }
            | BlockKind::Pwm { output_names, .. }
            | BlockKind::PhaseShiftPwm { output_names, .. } => {
                block_names.extend(output_names.iter().skip(1).cloned());
            }
            _ => {}
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
fn parse_signal(text: &str) -> Signal {
    match text.strip_prefix("prev:") {
        Some(name) => Signal::BlockPrev(name.to_string()),
        None => Signal::Block(text.to_string()),
    }
}

/// Splits `text` on top-level commas only — one inside a nested `[...]` (bracket depth > 0)
/// doesn't count — so `"[1,2],[3,4]"` splits into `["[1,2]", "[3,4]"]`, not four pieces. The
/// one primitive every Python-list-literal field below (`num=`/`den=`/`a=`/`b=`/`c=`/`points=`)
/// is built from, so a matrix's row separator and a vector's entry separator are the same
/// operation applied at a different nesting depth, not two different parsers.
fn split_top_level(text: &str) -> Vec<&str> {
    let mut depth = 0i32;
    let mut start = 0usize;
    let mut parts = Vec::new();
    for (i, c) in text.char_indices() {
        match c {
            '[' => depth += 1,
            ']' => depth -= 1,
            ',' if depth == 0 => {
                parts.push(text[start..i].trim());
                start = i + c.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(text[start..].trim());
    parts
}

/// Strips a field's required outer `[...]` — every list-valued field here is a real Python list
/// literal (`num=[1,2,3]`, `a=[[1,2],[3,4]]`), never the bare comma/semicolon/colon delimiters
/// an earlier version of this grammar used, so a Python list built from the same numbers can be
/// pasted into a `.cir` file (spaces after commas included: [`split_top_level`]/the `f64` parse
/// below both trim) without reformatting.
fn strip_brackets<'a>(
    text: &'a str,
    name: &str,
    field: &str,
    line_number: usize,
) -> Result<&'a str, String> {
    let trimmed = text.trim();
    trimmed
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .ok_or_else(|| {
            format!(
                "line {}: device '{name}' field '{field}' must be a Python-style list, e.g. \
                 '[1,2,3]' or '[[1,2],[3,4]]' (got '{text}')",
                line_number + 1
            )
        })
}

/// Parses a `[[<x>,<y>],[<x>,<y>],...]` point list (a `kind=pwc`/`kind=pwl` reference schedule,
/// or a `kind=table` lookup table) — a Python list of 2-element `[x, y]` lists, exactly the
/// shape `numpy.array(points)` would expect for an `Nx2` table. Sorted ascending by `x` on
/// return.
fn parse_xy_points(
    text: &str,
    name: &str,
    field: &str,
    line_number: usize,
) -> Result<Vec<(f64, f64)>, String> {
    let rows = parse_matrix_rows(text, name, field, line_number)?;
    let mut points = Vec::with_capacity(rows.len());
    for row in rows {
        let [x, y] = row.as_slice() else {
            return Err(format!(
                "line {}: device '{name}' field '{field}': each entry must be a 2-element \
                 '[x,y]' list (got '{row:?}')",
                line_number + 1
            ));
        };
        points.push((*x, *y));
    }
    points.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    Ok(points)
}

/// Parses a Python-style list of numbers, e.g. a `kind=statespace` block's `b=[1,2]`/`c=[1,0]`
/// vector or a `kind=tf` block's `num=[1,2]`/`den=[1,3,2]` coefficients.
fn parse_vector(
    text: &str,
    name: &str,
    field: &str,
    line_number: usize,
) -> Result<Vec<f64>, String> {
    let inner = strip_brackets(text, name, field, line_number)?;
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    split_top_level(inner)
        .into_iter()
        .map(|v| {
            v.parse::<f64>().map_err(|_| {
                format!(
                    "line {}: device '{name}' field '{field}' entry '{v}' is not a number",
                    line_number + 1
                )
            })
        })
        .collect()
}

/// Parses a `kind=statespace` block's `a` matrix: a Python-style list of row lists, e.g.
/// `a=[[1,2],[3,4]]` — exactly `numpy.array([[1,2],[3,4]])`'s own literal shape.
fn parse_matrix_rows(
    text: &str,
    name: &str,
    field: &str,
    line_number: usize,
) -> Result<Vec<Vec<f64>>, String> {
    let inner = strip_brackets(text, name, field, line_number)?;
    if inner.is_empty() {
        return Ok(Vec::new());
    }
    split_top_level(inner)
        .into_iter()
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
                // Every gate is block-driven -- gate=block ctrl=<name>, reading that block's
                // current output (>= 0.5 means on). No other gate= spelling exists: a
                // permanently-off gate is an explicit `Const(0.0)` wired through
                // `kind=sig2gate`, the same as any other gate.
                let gate = match fields.get("gate").map(String::as_str) {
                    Some("block") => GateSpec {
                        ctrl: get_str("ctrl")?,
                    },
                    Some(other) => {
                        return Err(format!(
                            "line {}: unknown gate spec '{other}' (only gate=block ctrl=<name> \
                             exists -- every gate is block-driven)",
                            line_number + 1
                        ))
                    }
                    None => {
                        return Err(format!(
                            "line {}: device '{name}' missing field 'gate' (gate=block \
                             ctrl=<name> -- every gate is block-driven, see this file's own \
                             module doc comment)",
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
            "time" => Kind::Block(BlockInstance {
                name: name.to_string(),
                kind: BlockKind::Time,
                inputs: Vec::new(),
            }),
            // "repeat=true" is optional on both "pwc" and "pwl" (default false, unchanged
            // hold-flat-past-the-end behavior) -- wraps time into the breakpoint list's own
            // [first, last) span once past the last point, making it periodic. See
            // `dae_runtime::block_graph`'s own doc comment on `BlockKind::Pwc`/`BlockKind::Pwl`
            // for why these are two distinct block kinds (interpolation style) rather than one
            // with a flag, and why "pwl" here means real piecewise-*linear* SPICE PWL semantics
            // while the older piecewise-*constant* block was renamed to "pwc" to free that name
            // up.
            "pwc" => {
                let points = parse_xy_points(&get_str("points")?, name, "points", line_number)?;
                let repeat = fields.get("repeat").map(|s| s == "true").unwrap_or(false);
                Kind::Block(BlockInstance {
                    name: name.to_string(),
                    kind: BlockKind::Pwc { points, repeat },
                    inputs: Vec::new(),
                })
            }
            "pwl" => {
                let points = parse_xy_points(&get_str("points")?, name, "points", line_number)?;
                let repeat = fields.get("repeat").map(|s| s == "true").unwrap_or(false);
                Kind::Block(BlockInstance {
                    name: name.to_string(),
                    kind: BlockKind::Pwl { points, repeat },
                    inputs: Vec::new(),
                })
            }
            // "sinwave"/"pulsewave"/"expwave"/"sffmwave": the electrical domain's other four
            // time-varying source forms (see `elspice_mna::TransientFunction`'s own doc comment
            // for the exact formula each implements), reused directly rather than
            // reimplemented, with the same field names/order/defaults as ngspice/Xyce's own
            // SIN()/PULSE()/EXP()/SFFM() -- so a `kind=sinwave ...` reference schedule and a
            // `V1 a 0 SIN(...)` source built from the same numbers are bit-for-bit the same
            // waveform. Named "...wave" rather than the bare SPICE keyword specifically to
            // avoid colliding with the pre-existing `kind=sin`/`kind=exp` waveform-arithmetic
            // *functions* (`sin(x)`/`exp(x)` of an input signal, see the `MathFn1` fallback
            // dispatch below) -- "pulse"/"sffm" have no such collision today, but are named the
            // same way for consistency across the family rather than only where forced to.
            "sinwave" => {
                let get_opt = |key: &str, default: f64| -> f64 {
                    fields
                        .get(key)
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(default)
                };
                Kind::Block(BlockInstance {
                    name: name.to_string(),
                    kind: BlockKind::Waveform(TransientFunction::Sin {
                        v0: get_opt("v0", 0.0),
                        va: get("va")?,
                        freq: get("freq")?,
                        td: get_opt("td", 0.0),
                        theta: get_opt("theta", 0.0),
                        phase: get_opt("phase", 0.0),
                    }),
                    inputs: Vec::new(),
                })
            }
            "pulsewave" => {
                let get_opt = |key: &str, default: f64| -> f64 {
                    fields
                        .get(key)
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(default)
                };
                Kind::Block(BlockInstance {
                    name: name.to_string(),
                    kind: BlockKind::Waveform(TransientFunction::Pulse {
                        v1: get("v1")?,
                        v2: get("v2")?,
                        td: get_opt("td", 0.0),
                        tr: get_opt("tr", 0.0),
                        tf: get_opt("tf", 0.0),
                        pw: get_opt("pw", f64::MAX / 4.0),
                        per: get_opt("per", f64::MAX / 4.0),
                    }),
                    inputs: Vec::new(),
                })
            }
            "expwave" => {
                let get_opt = |key: &str, default: f64| -> f64 {
                    fields
                        .get(key)
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(default)
                };
                let td1 = get_opt("td1", 0.0);
                // td2 defaults to "effectively never" (matching pulsewave's own pw/per
                // defaults just above), NOT td1 -- TransientFunction::Exp treats `t < td2` as
                // "still in the rise phase," so a naive td1 default would make every omitted-
                // td2 call fall straight into the *fall* phase at t=0 instead of never falling
                // at all (a real bug caught by this file's own dae-runtime-level test).
                Kind::Block(BlockInstance {
                    name: name.to_string(),
                    kind: BlockKind::Waveform(TransientFunction::Exp {
                        v1: get("v1")?,
                        v2: get("v2")?,
                        td1,
                        tau1: get_opt("tau1", 1.0),
                        td2: get_opt("td2", f64::MAX / 4.0),
                        tau2: get_opt("tau2", 1.0),
                    }),
                    inputs: Vec::new(),
                })
            }
            "sffmwave" => {
                let get_opt = |key: &str, default: f64| -> f64 {
                    fields
                        .get(key)
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(default)
                };
                Kind::Block(BlockInstance {
                    name: name.to_string(),
                    kind: BlockKind::Waveform(TransientFunction::Sffm {
                        v0: get_opt("v0", 0.0),
                        va: get("va")?,
                        fc: get("fc")?,
                        mdi: get_opt("mdi", 0.0),
                        fs: get("fs")?,
                    }),
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
                let error_input = parse_signal(&get_str("in")?);
                let (clamp, inputs) = match (fields.get("clamp_lo_in"), fields.get("clamp_hi_in")) {
                    (Some(lo), Some(hi)) => (
                        PidClamp::Dynamic,
                        vec![error_input, parse_signal(lo), parse_signal(hi)],
                    ),
                    (None, None) => (
                        PidClamp::Fixed(get("clamp_lo")?, get("clamp_hi")?),
                        vec![error_input],
                    ),
                    _ => {
                        return Err(format!(
                            "line {}: device '{name}': 'clamp_lo_in'/'clamp_hi_in' must both be \
                             given together (dynamic clamp) or both omitted (fixed clamp= \
                             clamp_lo/clamp_hi)",
                            line_number + 1
                        ))
                    }
                };
                Kind::Block(BlockInstance {
                    name: name.to_string(),
                    kind: BlockKind::Pid { pid, clamp },
                    inputs,
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
            // "pwm"/"pspwm": fixed-frequency and frequency+phase+duty active-high-complementary
            // PWM modulators (see `dae_runtime::BlockKind::Pwm`/`PhaseShiftPwm`'s own doc
            // comments for the full design) -- both default `red`/`fed` (dead time, seconds) to
            // 0.0, the ideal gap-free/overlap-free complementary pair.
            "pwm" => {
                let output_names = match fields.get("outputs") {
                    Some(names) => {
                        let names: Vec<String> = names.split(',').map(str::to_string).collect();
                        if names.len() != 2 {
                            return Err(format!(
                                "line {}: device '{name}': 'outputs' needs exactly 2 entries \
                                 (main, complement; got {})",
                                line_number + 1,
                                names.len()
                            ));
                        }
                        names
                    }
                    None => vec![name.to_string(), format!("{name}_comp")],
                };
                Kind::Block(BlockInstance {
                    name: name.to_string(),
                    kind: BlockKind::Pwm {
                        freq_hz: get("freq")?,
                        red: fields
                            .get("red")
                            .map(|s| s.parse::<f64>())
                            .transpose()
                            .map_err(|_| {
                                format!(
                                    "line {}: device '{name}' field 'red' is not a number",
                                    line_number + 1
                                )
                            })?
                            .unwrap_or(0.0),
                        fed: fields
                            .get("fed")
                            .map(|s| s.parse::<f64>())
                            .transpose()
                            .map_err(|_| {
                                format!(
                                    "line {}: device '{name}' field 'fed' is not a number",
                                    line_number + 1
                                )
                            })?
                            .unwrap_or(0.0),
                        output_names,
                    },
                    inputs: vec![parse_signal(&get_str("in")?)],
                })
            }
            "pspwm" => {
                let osc = Vco::new(get("f_min")?, get("f_max")?);
                let inputs: Vec<Signal> = get_str("inputs")?.split(',').map(parse_signal).collect();
                if inputs.len() != 3 {
                    return Err(format!(
                        "line {}: device '{name}' kind='pspwm' needs 3 inputs \
                         (freq,phase,duty; got {})",
                        line_number + 1,
                        inputs.len()
                    ));
                }
                let output_names = match fields.get("outputs") {
                    Some(names) => {
                        let names: Vec<String> = names.split(',').map(str::to_string).collect();
                        if names.len() != 2 {
                            return Err(format!(
                                "line {}: device '{name}': 'outputs' needs exactly 2 entries \
                                 (main, complement; got {})",
                                line_number + 1,
                                names.len()
                            ));
                        }
                        names
                    }
                    None => vec![name.to_string(), format!("{name}_comp")],
                };
                let get_opt = |key: &str| -> Result<f64, String> {
                    fields
                        .get(key)
                        .map(|s| s.parse::<f64>())
                        .transpose()
                        .map_err(|_| {
                            format!(
                                "line {}: device '{name}' field '{key}' is not a number",
                                line_number + 1
                            )
                        })
                        .map(|v| v.unwrap_or(0.0))
                };
                Kind::Block(BlockInstance {
                    name: name.to_string(),
                    kind: BlockKind::PhaseShiftPwm {
                        osc,
                        red: get_opt("red")?,
                        fed: get_opt("fed")?,
                        output_names,
                    },
                    inputs,
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
                let points = parse_xy_points(&get_str("points")?, name, "points", line_number)?;
                Kind::Block(BlockInstance {
                    name: name.to_string(),
                    kind: BlockKind::Table(points),
                    inputs: vec![parse_signal(&get_str("in")?)],
                })
            }
            "probe" => {
                let target = match (fields.get("node"), fields.get("branch")) {
                    (Some(node), None) => ProbeTarget::Voltage(node.clone()),
                    (None, Some(branch)) => ProbeTarget::Current(branch.clone()),
                    (Some(_), Some(_)) => {
                        return Err(format!(
                            "line {}: device '{name}': 'node' and 'branch' are mutually \
                             exclusive (a probe reads either a node voltage or a branch \
                             current, never both)",
                            line_number + 1
                        ))
                    }
                    (None, None) => {
                        return Err(format!(
                            "line {}: device '{name}' kind='probe' needs 'node=<name>' (reads \
                             V(node)) or 'branch=<name>' (reads I(branch))",
                            line_number + 1
                        ))
                    }
                };
                Kind::Block(BlockInstance {
                    name: name.to_string(),
                    kind: BlockKind::Probe(target),
                    inputs: Vec::new(),
                })
            }
            "sig2gate" => Kind::Block(BlockInstance {
                name: name.to_string(),
                kind: BlockKind::Sig2Gate,
                inputs: vec![parse_signal(&get_str("in")?)],
            }),
            "sig2voltage" => Kind::Block(BlockInstance {
                name: name.to_string(),
                kind: BlockKind::Sig2Voltage,
                inputs: vec![parse_signal(&get_str("in")?)],
            }),
            "sig2current" => Kind::Block(BlockInstance {
                name: name.to_string(),
                kind: BlockKind::Sig2Current,
                inputs: vec![parse_signal(&get_str("in")?)],
            }),
            "clarke" | "clarkeinv" | "park" | "parkinv" | "clarkepark" | "clarkeparkinv" => {
                let ct = match kind {
                    "clarke" => CoordinateTransform::Clarke,
                    "clarkeinv" => CoordinateTransform::ClarkeInv,
                    "park" => CoordinateTransform::Park,
                    "parkinv" => CoordinateTransform::ParkInv,
                    "clarkepark" => CoordinateTransform::ClarkePark,
                    "clarkeparkinv" => CoordinateTransform::ClarkeParkInv,
                    _ => unreachable!("matched above"),
                };
                let inputs: Vec<Signal> = get_str("inputs")?.split(',').map(parse_signal).collect();
                if inputs.len() != ct.input_count() {
                    return Err(format!(
                        "line {}: device '{name}' kind='{kind}' needs {} inputs (got {})",
                        line_number + 1,
                        ct.input_count(),
                        inputs.len()
                    ));
                }
                let output_names = match fields.get("outputs") {
                    Some(names) => {
                        let names: Vec<String> = names.split(',').map(str::to_string).collect();
                        if names.len() != 3 {
                            return Err(format!(
                                "line {}: device '{name}': 'outputs' needs exactly 3 entries \
                                 (got {})",
                                line_number + 1,
                                names.len()
                            ));
                        }
                        names
                    }
                    // Default: block's own name aliases the first (primary) output, same as
                    // `kind=cscript`'s default; the other two get readable auto-generated names
                    // from this transform's own conventional output names (e.g. `<name>_beta`).
                    None => {
                        let suffixes = ct.output_names();
                        std::iter::once(name.to_string())
                            .chain(suffixes[1..].iter().map(|s| format!("{name}_{s}")))
                            .collect()
                    }
                };
                Kind::Block(BlockInstance {
                    name: name.to_string(),
                    kind: BlockKind::CoordinateTransform {
                        kind: ct,
                        output_names,
                    },
                    inputs,
                })
            }
            "pmsm" => {
                let pmsm = continuous_blocks::Pmsm::new(
                    get("r_s")?,
                    get("l_d")?,
                    get("l_q")?,
                    get("lambda_pm")?,
                    get("pole_pairs")?,
                    get("inertia")?,
                    get("friction")?,
                );
                let inputs: Vec<Signal> = get_str("inputs")?.split(',').map(parse_signal).collect();
                if inputs.len() != 3 {
                    return Err(format!(
                        "line {}: device '{name}' kind='pmsm' needs 3 inputs (vd,vq,t_load; \
                         got {})",
                        line_number + 1,
                        inputs.len()
                    ));
                }
                let output_names = match fields.get("outputs") {
                    Some(names) => {
                        let names: Vec<String> = names.split(',').map(str::to_string).collect();
                        if names.len() != 4 {
                            return Err(format!(
                                "line {}: device '{name}': 'outputs' needs exactly 4 entries \
                                 (got {})",
                                line_number + 1,
                                names.len()
                            ));
                        }
                        names
                    }
                    // Default: block's own name aliases the first (primary) output (`id`), same
                    // convention as `kind=cscript`/`kind=clarke`; the other three get readable
                    // auto-generated names.
                    None => {
                        let suffixes = ["id", "iq", "omega_m", "theta_e"];
                        std::iter::once(name.to_string())
                            .chain(suffixes[1..].iter().map(|s| format!("{name}_{s}")))
                            .collect()
                    }
                };
                Kind::Block(BlockInstance {
                    name: name.to_string(),
                    kind: BlockKind::Pmsm { pmsm, output_names },
                    inputs,
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
