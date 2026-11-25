//! Thin runner for `general-simulator`: `general-simulator <netlist> [--devices <file>] --mode
//! {dc|transient} [--tfinal T] [--dt DT | --dt-max/--dt-min/--dt-init/--reltol/--abstol]
//! [--max-steps N]` prints a CSV waveform (`t,V(node1),V(node2),...`) to stdout, one row per
//! resolved timestep (a single row for `--mode dc`). `--dt` fixes the step size; omit it for
//! adaptive step-size control instead (the default — see `dae_runtime::TimeStep`/
//! `AdaptiveConfig`'s own doc comments for the algorithm), with `--dt-max`/`--dt-min`/
//! `--dt-init`/`--reltol`/`--abstol` overriding individual defaults derived from `--tfinal`.
//! `--max-steps` caps how many accepted adaptive steps one run may take (default
//! [`dae_runtime::ADAPTIVE_STEP_HARD_CAP`]) before aborting with `DaeError::AdaptiveStepStalled`
//! instead of growing its in-memory trace without limit — see that error's own doc comment.
//! Any `kind=measure` results write to `<netlist stem>.log`, never stdout/stderr.
//!
//! **The netlist is the one file** — PWL device parameters and any controller block graph
//! live directly inside it, the same way a real SPICE deck is self-contained, not split across
//! files by convention. **This binary no longer parses any of that itself** — `general-mna`'s
//! `build_system` does, via `general-spice-core`'s real grammar (see its own
//! `docs/GRAMMAR.md` §12), which now has a genuine first-class syntax for a `kind=ideal_switch
//! r_on=...`/block-graph line, no `*`-comment disguise needed:
//!
//! ```text
//! D1 kind=ideal_diode g_breakdown=0 v_breakdown=-100 g_off=0 v_f=0.7 g_on=1
//! ONVAL kind=const value=1
//! ONGATE kind=sig2phys domain=voltage in=ONVAL
//! D2 kind=ideal_switch r_on=0.1 body_g_breakdown=0 body_v_breakdown=-100 body_g_off=0 body_v_f=0.5 body_g_on=5 gate=block ctrl=ONGATE
//! DUTY kind=const value=0.6
//! PWM1 kind=pwm freq=10000 in=DUTY outputs=PWM1_ON,PWM1_OFF
//! PWM1G kind=sig2phys domain=voltage in=PWM1_ON
//! D3 kind=ideal_switch r_on=0.1 body_g_breakdown=0 body_v_breakdown=-100 body_g_off=0 body_v_f=0.5 body_g_on=5 gate=block ctrl=PWM1G
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
//! Every ideal switch must declare the same `r_on` — `dae-runtime`'s switch mechanism uses one shared
//! on-resistance per call (see `dae_runtime::solve_dc_with_ideal_switches`'s doc comment).
//!
//! ## Wiring a controller into a gate
//!
//! **There is no separate "closed-loop mode," and no non-block-driven gate at all.** Every
//! ideal switch's gate is `gate=block ctrl=<sig2phys-voltage-name>` — always resolved the same way, every
//! step, by reading the named block's current output (`>= 0.5` means on). Even a permanently-on
//! or permanently-off gate is an ordinary block (`kind=const value=1`, wrapped in a
//! `kind=sig2phys domain=voltage` like any other gate target — an ideal switch's gate is itself
//! a voltage, so it shares the same converter a `V`-source's own magnitude uses, not a
//! dedicated gate-only type), not a special fixed-state escape hatch — see
//! "Physical/signal-domain converters" below for why every `ctrl=` target must specifically be
//! a `domain=voltage` `sig2phys` converter.
//! `continuous-blocks` supplies the vocabulary the device file wires
//! together — `const`, `time` (zero-input, outputs the current step's own simulated time — the
//! standard "clock" source, needed to build a `sin(2*pi*f*t)`-style signal via `MathFn1`/`Gain`
//! since no block otherwise sees `t` directly), `pwc`/`pwl` (piecewise-constant/-linear
//! sources, e.g. a reference schedule), `sum` (an error junction with explicit `+`/`-` signs),
//! `gain`, `pid`, `statespace` (arbitrary `(A,B,C,D)`), `tf` (a rational `N(s)/D(s)`), `vco` (a
//! bare oscillator ramp), and the two PWM modulators this vocabulary is ultimately *for* —
//! `pwm`/`pspwm`, see their own section below. Whether a graph happens to read the circuit's own
//! state back (through a `kind=phys2sig`, see below,
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
//! **not the same kind of thing** and cannot be wired together directly — the same rule a real
//! block-diagram tool enforces with its own physical/signal converter blocks, applied here at
//! the netlist level (a companion UI is intended to enforce the same rule visually later; this
//! grammar is the ground truth). There are two converters, one per
//! crossing direction (read vs. write) — the write direction, `sig2phys` (parameterized by
//! `domain=voltage`/`domain=current`), covers both a gate command and a source's own magnitude,
//! since an ideal switch's gate is itself a voltage rather than a distinct discrete-actuation
//! signal domain:
//!
//! - `kind=phys2sig node=<name>` (reads `V(node)`) or `kind=phys2sig branch=<name>` (reads
//!   `I(branch)`, mutually exclusive with `node=`) is the **only** way a circuit quantity enters
//!   the signal domain. Zero inputs (a source block, like `const`/`time`); reference its output
//!   afterward exactly like any other block's, e.g. `ERR kind=sum inputs=REF,VOUT_PROBE
//!   signs=1,-1` where `VOUT_PROBE kind=phys2sig node=vout` was declared earlier.
//! - `kind=sig2phys domain=voltage in=<signal>` / `kind=sig2phys domain=current in=<signal>`
//!   are the **only** legal way a signal-domain block drives an independent voltage/current
//!   source's own magnitude, *or* an ideal switch's gate. For a source: name the converter
//!   block directly as that source's own literal value in the netlist, e.g. `V1 a 0 VDRV` where
//!   `VDRV kind=sig2phys domain=voltage in=CTRL` was declared earlier (`general-mna` already
//!   accepts a bare symbol there; no change was needed on that side). `V` sources need
//!   `domain=voltage`, `I` sources need `domain=current` — a mismatch, or naming any other kind
//!   of block, is rejected (`dae_runtime::DaeError::SourceNotSig2PhysicalConverter`) before any
//!   step runs. For a gate: `gate=block ctrl=<name>` requires `<name>` to be a `domain=voltage`
//!   `sig2phys` converter too, not a raw `pid`/`vco`/`pwm`/`hysteresis`/etc. block directly, and
//!   not a `domain=current` `sig2phys` either
//!   (`dae_runtime::DaeError::GateTargetNotSig2Voltage` otherwise). Purely an identity
//!   pass-through numerically in both roles — its whole purpose is marking, in the netlist
//!   text, exactly where a signal stops being "a number a controller computed" and starts being
//!   a physical voltage, whether that voltage sources a node or gates a switch. This closes
//!   a real, previously-open gap: without it, a block could only ever *observe* the circuit
//!   (via `kind=phys2sig`), never load or drive it — see an internal buck-converter/DC-motor-cascade
//!   experiment for the concrete limitation this fixes.
//!   Those two are the *only* ways to consume a converter, and both are by **name**: a
//!   `kind=sig2phys` block has no terminals and stamps nothing, so it can never be *wired* into
//!   the netlist. Using its name as a node on an element line (`R1 VDRV 0 1k`) used to build and
//!   solve without complaint and silently report `V(VDRV) = 0` — an ordinary undriven node — next
//!   to the block's own correct, non-zero output column; that is now rejected up front, in both
//!   `--mode dc` and `--mode transient`, with `dae_runtime::DaeError::Sig2PhysUsedAsCircuitNode`
//!   naming the converter, the offending element, and the node token as written.
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
//! Both feed `gate=block ctrl=<sig2phys-voltage-name>` on each switch — one `domain=voltage`
//! `sig2phys` wrapping `main`, another wrapping `complement`, for a true half-bridge leg's two
//! switches; a topology with only one actively-driven switch (a buck's own high-side,
//! freewheeling through a diode) just leaves the `complement` output unwired. **Every `ctrl=`
//! target must resolve to a `domain=voltage` `kind=sig2phys` converter** (see
//! "Physical/signal-domain converters" above), never the raw `pwm`/`pspwm`/`hysteresis`/etc.
//! block directly.
//!
//! Example — a frequency-modulated half-bridge PID (LLC-family converters regulate by
//! switching frequency, not PWM duty, unlike buck/boost) with a reference step test, as it
//! would appear inside the `.cir` file alongside the actual circuit elements (`V1`, `Lr`, ...):
//!
//! ```text
//! V1 vin 0 400
//! * D1 kind=ideal_switch r_on=0.01 body_g_breakdown=0 body_v_breakdown=-1e6 body_g_off=1e-6 body_v_f=1e6 body_g_on=0 gate=block ctrl=LEG_MAIN_G
//! * D2 kind=ideal_switch r_on=0.01 body_g_breakdown=0 body_v_breakdown=-1e6 body_g_off=1e-6 body_v_f=1e6 body_g_on=0 gate=block ctrl=LEG_COMP_G
//! ... (Lr, Cr, transformer, rectifier, Cout, Rout -- ordinary SPICE elements)
//! * VOUT_PROBE kind=phys2sig node=vout
//! * REF     kind=pwc points=[[0,20],[0.014,17]]
//! * ERR     kind=sum inputs=REF,VOUT_PROBE signs=1,-1
//! * PID1    kind=pid kp=800 ki=4e6 kd=0 n=1000 clamp_lo=-15000 clamp_hi=15000 in=ERR
//! * FNOM    kind=const value=115000
//! * FREQ    kind=sum inputs=FNOM,PID1 signs=1,-1
//! * ZEROPH  kind=const value=0
//! * HALFDUTY kind=const value=0.48
//! * LEG     kind=pspwm f_min=100000 f_max=130000 inputs=FREQ,ZEROPH,HALFDUTY outputs=LEG_MAIN,LEG_COMP
//! * LEG_MAIN_G kind=sig2phys domain=voltage in=LEG_MAIN
//! * LEG_COMP_G kind=sig2phys domain=voltage in=LEG_COMP
//! ```
//!
//! A real SPICE tool opening this file sees eleven ordinary comment lines and an otherwise
//! unremarkable LLC deck. `general-simulator-cli some.cir --mode transient` (no `--devices`) sees the
//! complete closed loop.
//!
//! `--mode dc` cannot resolve any gate at all now that every gate is block-driven: a DC
//! operating point has no notion of the time-stepped state a `Pid`/`Vco`/`Pwm`/`PhaseShiftPwm`
//! block carries, so any netlist with an ideal switch needs `--mode transient`.
//!
//! See internal closed-loop LLC-vs-Xyce/ngspice and buck underdamped-resonance-filter
//! experiments for full worked examples this syntax was built for.

mod finite;
mod measure;
mod raw_format;
mod rules;
mod signal_meta;
mod static_checks;

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use dae_runtime::checkpoint::Checkpoint;
use dae_runtime::{
    simulate_transient_streamed, simulate_transient_with_blocks_checkpointed, solve_dc,
    AdaptiveConfig, BlockInstance, BlockKind, CheckpointControl, CheckpointSink, DaeError,
    GateBinding, TimeStep, ADAPTIVE_STEP_HARD_CAP,
};
use general_spice_core::Dialect;
use pwl_devices::{IdealDiode, IdealSwitch};

/// The waveform this run produced, in a serialization-agnostic shape: `headers[0]` is always the
/// time/sweep column, `rows[point][column]` mirrors it. Both `print_csv` (the existing, default
/// output -- unchanged byte-for-byte from before this module existed) and
/// `raw_format::write_raw` (the new `--format raw` alternative) are built from exactly this, so
/// "same data, different serialization" (see `book/user-guide/src/reading-output.md`) is
/// structural rather than something the two writers merely happen to agree on.
#[derive(Clone)]
struct Waveform {
    headers: Vec<String>,
    rows: Vec<Vec<f64>>,
}

/// Output format for the resolved waveform. CSV (the original, and still the default, for
/// backward compatibility with this project's own existing tests and worked examples) prints to
/// stdout exactly as before; `Raw` writes a SPICE rawfile (see `raw_format`'s own doc comment) to
/// `--out <path>`, since a binary format has no sensible terminal rendering the way CSV does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum OutputFormat {
    Csv,
    Raw,
    /// A `.gsim_samples` container (see the `gsim-samples` crate). The only
    /// one of the three a viewer can read without loading the run: signals
    /// are addressable one at a time, time windows are addressable, and a
    /// min/max pyramid means the points read are set by a screen's budget
    /// rather than by the length of the run. CSV and the rawfile are both
    /// row-major, so neither can answer "these five signals over this
    /// window" without a scan from byte zero.
    Samples,
}

/// A `DaeError` as a line a user can act on.
///
/// Every variant is a `{e:?}` dump by default, which keeps the variant name greppable in stderr
/// and is the right default for an error whose fields already read as a sentence. Two do not:
/// the one a silent failure used to look like (`Sig2PhysUsedAsCircuitNode`, spelled out at its
/// own call site below), and this one -- a run that outlasts its capture is the ordinary mistake
/// with a file-backed source, and `covered: 0.001999, t_final: 0.003` makes a reader work out
/// what to do about it. The variant name is kept in front either way.
fn dae_message(error: &DaeError) -> String {
    match error {
        DaeError::WaveCaptureTooShort {
            block,
            source,
            covered,
            t_final,
        } => format!(
            "{error:?}: '{block}' reads '{source}', which ends at {covered:e} s, but the run \
             goes to {t_final:e} s. Past its last sample a capture holds its final value, so \
             the rest of the run would be driven by a flat line -- which is what `after=error` \
             exists to refuse. Shorten the run to {covered:e} s, capture more data, or choose \
             `after=hold`/`repeat`/`zero` if a held, repeated or zero tail is what you meant."
        ),
        other => format!("{other:?}"),
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

/// What the process returns when rules took part.
///
/// The specification asks for `1 = warnings`. That is not taken here, and the reason is worth
/// stating: `1` has always meant "this run failed" for every script and for the GUI's own
/// solver discovery, so reusing it for a run that completed and merely warned would silently
/// reclassify every existing failure. The codes below add to the contract instead of rewriting
/// it -- a rule-free netlist still returns 0 or 1 exactly as before.
mod exit {
    /// Nothing to report, or nothing at or above `--fail-on` (default `error`).
    pub const OK: u8 = 0;
    /// A rule fired at or above `--fail-on` — by default `error` or `fatal`.
    pub const RULE_ERROR: u8 = 2;
    /// A rule ended the run.
    pub const STOPPED_BY_RULE: u8 = 3;
    /// The solved state went non-finite and the run was abandoned.
    ///
    /// Its own code rather than `STOPPED_BY_RULE`, because the two mean opposite things to
    /// whatever reads them: 3 is a rule doing its job on a run that behaved as the deck said
    /// it would, and 4 is a run with no result at all.
    pub const NOT_FINITE: u8 = 4;
}

fn run() -> Result<u8, String> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        return Err(usage());
    }
    // Answered before `args[1]` is read as a path, because that is exactly what used to
    // happen to them: every argument is a netlist path to this CLI, so `--version` came
    // back as `error: reading --version: No such file or directory` with status 1. That
    // broke anything probing the binary to see whether it works -- the GUI's own solver
    // discovery among them -- and made `--help` useless to a first-time user.
    //
    // Both go to stdout and exit 0: asking for help is not a misuse. `usage()` stays on
    // stderr with a failing status, for arguments that really are wrong.
    match args[1].as_str() {
        "--help" | "-h" => {
            println!("{}", help());
            return Ok(0);
        }
        "--version" | "-V" => {
            println!("{}", version_line());
            return Ok(0);
        }
        // A leading dash in first position is a mistyped flag, not a file. Without this it
        // reached the file open and came back as
        // `error: reading --tfinal: No such file or directory`, which names the wrong
        // problem: the netlist is missing *and* the flag is in the wrong place. A path
        // that genuinely starts with a dash is still reachable as `./-odd.cir`.
        other if other.starts_with('-') => {
            return Err(format!(
                "unrecognized argument '{other}', or a flag given before the netlist.\n\
                 The netlist path comes first: general-simulator <netlist> [OPTIONS]\n\
                 Run `general-simulator --help` for the full list."
            ));
        }
        _ => {}
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
    let mut max_steps: Option<usize> = None;
    let mut format = "csv".to_string();
    let mut out_path: Option<String> = None;
    let mut events_jsonl: Option<String> = None;
    let mut events_text: Option<String> = None;
    let mut fail_on = gsim_rules::Severity::Error;
    let mut out_every: usize = 1;
    let mut checkpoint_out: Option<String> = None;
    let mut checkpoint_every: Option<f64> = None;
    let mut resume: Option<String> = None;

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
            "--max-steps" => {
                let v: usize = args
                    .get(i + 1)
                    .ok_or("--max-steps needs a value")?
                    .parse()
                    .map_err(|_| "bad --max-steps".to_string())?;
                max_steps = Some(v);
                i += 2;
            }
            "--out-every" => {
                let v: usize = args
                    .get(i + 1)
                    .ok_or("--out-every needs a value")?
                    .parse()
                    .map_err(|_| "bad --out-every".to_string())?;
                if v == 0 {
                    return Err("--out-every must be at least 1".to_string());
                }
                out_every = v;
                i += 2;
            }
            "--format" => {
                format = args.get(i + 1).ok_or("--format needs a value")?.clone();
                i += 2;
            }
            "--checkpoint-out" => {
                checkpoint_out = Some(
                    args.get(i + 1)
                        .ok_or("--checkpoint-out needs a value")?
                        .clone(),
                );
                i += 2;
            }
            "--checkpoint-every" => {
                let v = parse_f64("--checkpoint-every", &mut i)?;
                if v <= 0.0 {
                    return Err("--checkpoint-every must be positive".to_string());
                }
                checkpoint_every = Some(v);
            }
            "--resume" => {
                resume = Some(args.get(i + 1).ok_or("--resume needs a value")?.clone());
                i += 2;
            }
            "--out" => {
                out_path = Some(args.get(i + 1).ok_or("--out needs a value")?.clone());
                i += 2;
            }
            // The two event files, separately: a tool wants the JSONL and a person wants the
            // text, and moving one should not hand the other a file nobody reads.
            "--events" => {
                events_jsonl = Some(args.get(i + 1).ok_or("--events needs a value")?.clone());
                i += 2;
            }
            "--events-text" => {
                events_text = Some(
                    args.get(i + 1)
                        .ok_or("--events-text needs a value")?
                        .clone(),
                );
                i += 2;
            }
            // Which severity makes the process fail. The default is `error`, which is what
            // the exit-code contract described before this flag existed, so a caller that
            // does not pass it sees no change.
            "--fail-on" => {
                let value = args.get(i + 1).ok_or("--fail-on needs a value")?;
                fail_on = match value.to_ascii_lowercase().as_str() {
                    "info" => gsim_rules::Severity::Info,
                    "warning" | "warn" => gsim_rules::Severity::Warning,
                    "error" => gsim_rules::Severity::Error,
                    "fatal" => gsim_rules::Severity::Fatal,
                    other => {
                        return Err(format!(
                            "--fail-on takes info, warning, error or fatal, not '{other}'"
                        ))
                    }
                };
                i += 2;
            }
            other => return Err(format!("unrecognized argument '{other}'\n{}", usage())),
        }
    }

    let output_format = match format.as_str() {
        "csv" => OutputFormat::Csv,
        "raw" => OutputFormat::Raw,
        "samples" => OutputFormat::Samples,
        other => {
            return Err(format!(
                "unknown --format '{other}' (expected 'csv', 'raw' or 'samples')"
            ))
        }
    };
    // A binary rawfile can't sensibly print to a terminal the way CSV does, so `--format raw`
    // always needs a file destination -- either an explicit `--out <path>`, or (the common case,
    // so a plain `general-simulator some.cir --mode transient --format raw` just works) the
    // netlist's own filename stem with a `.raw` extension, next to the input file. `--out` given
    // alongside `--format csv` is accepted too (it just means "write the CSV to a file instead
    // of stdout" is *not* supported -- CSV always goes to stdout, unchanged from before this
    // flag existed) and is otherwise ignored, rather than rejected, since silently ignoring an
    // extra flag that doesn't apply is friendlier than erroring on a harmless combination.
    let raw_out_path: Option<PathBuf> = if output_format == OutputFormat::Raw {
        Some(match &out_path {
            Some(p) => PathBuf::from(p),
            None => default_raw_path(netlist_path),
        })
    } else {
        None
    };
    // `--out-every` and `--format samples` are both answers to "this run is too big to look
    // at", and composing them is the one combination that is silently wrong. A stride drops
    // the samples between the points it keeps; the pyramid keeps the extremes of every
    // interval. Striding first throws away exactly what the pyramid exists to preserve, and
    // what comes out *looks* complete -- a container with a full pyramid, built over a run
    // that has already had most of it removed. Nothing downstream can tell.
    //
    // So this is refused rather than warned about: there is no reading of the two flags
    // together under which the user gets what either one promises. See `--out-every`'s own
    // help text for which to reach for (#107).
    if output_format == OutputFormat::Samples && out_every > 1 {
        return Err(format!(
            "--out-every {out_every} cannot be combined with --format samples.\n\
             \n\
             A stride keeps one point per interval and discards the rest. The samples format \n\
             already stores the minimum and maximum of every interval, so a viewer reads a \n\
             screen's worth of points from the whole run -- which is the same problem solved \n\
             without losing anything. Applied together, the stride removes the peaks before \n\
             the pyramid can record them, and the result is indistinguishable from a \n\
             complete file.\n\
             \n\
             Drop --out-every and keep --format samples, or drop --format samples if you \n\
             genuinely want every Nth point and nothing between them."
        ));
    }

    // Same rule as the rawfile, for the same reason: a binary container has
    // nothing to show a terminal, so it needs a destination, and defaulting
    // to the netlist's own stem keeps `--format samples` a one-flag change.
    let samples_out_path: Option<PathBuf> = if output_format == OutputFormat::Samples {
        Some(match &out_path {
            Some(p) => PathBuf::from(p),
            None => PathBuf::from(netlist_path).with_extension("gsim_samples"),
        })
    } else {
        None
    };

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
    // `kind=measure` lines (see `measure`'s own module doc comment for the full architecture
    // rationale) are recognized and stripped here, before `general_mna::build_system` ever sees
    // them -- `general-mna` has no `kind=measure` entry in its own dispatch and would reject it
    // as an unknown device kind otherwise. The stripped copy (blank lines in place of the
    // original ones, so every other statement keeps its own line number) is what actually goes
    // to `build_system`; the collected `MeasureSpec`s are evaluated once the transient trace is
    // complete, see below.
    let (devices_for_build, measurements) = measure::extract(&devices_source, dialect)
        .map_err(|e| format!("parsing kind=measure declarations: {e}"))?;
    // `kind=rule` cards come out for the same reason, and for one more: the deck hash a
    // checkpoint is keyed by is taken over the parsed statements, so a rule left in them would
    // mean that editing a limit invalidates the checkpoint of the run that limit just stopped.
    // Stripping them here is what lets a run stop on a rule, have the rule adjusted, and resume.
    let rules::Extracted {
        netlist: devices_for_build,
        rules: device_rules,
        ports: device_ports,
        switches: device_switch_channels,
        diodes: device_diode_curves,
    } = rules::extract(&devices_for_build, dialect).map_err(|e| {
        format!("parsing kind=rule cards, kind=port declarations and device ratings: {e}")
    })?;
    // The solver re-parses the netlist *itself*, and the deck hash a checkpoint is keyed by is
    // taken over those statements -- so the cards have to be gone from this copy too, not only
    // from the one `build_system` sees. Without this, editing a limit changes the hash, and the
    // checkpoint written by the run that limit just stopped refuses to load into the deck that
    // differs from it by one number.
    let rules::Extracted {
        netlist,
        rules: netlist_rules,
        ports: netlist_ports,
        switches: netlist_switch_channels,
        diodes: netlist_diode_curves,
    } = rules::extract(&netlist, dialect).map_err(|e| {
        format!("parsing kind=rule cards, kind=port declarations and device ratings: {e}")
    })?;
    // With no `--devices`, both extractions ran over the same text and found the same cards;
    // with one, a rule may sit in either file and both sets count.
    let declared_rules = if devices_path.is_some() {
        let mut all = netlist_rules;
        all.extend(device_rules);
        all
    } else {
        device_rules
    };
    // Ports are merged the same way, and for the same reason: with `--devices` a port may be
    // declared in either file. A name declared in both is the devices file's, which is the same
    // precedence `extract` itself applies within one file.
    let declared_ports = if devices_path.is_some() {
        let mut all = netlist_ports;
        all.extend(device_ports);
        all
    } else {
        device_ports
    };
    // Switch channels merge the same way. With `--devices` the device line and its
    // `kind=ideal_switch` line are normally in the same file, so a channel assembled from one
    // file is the usual case and a half from each is simply absent -- which makes
    // `P_cond(NAME)` unreadable rather than wrong.
    let switch_channels = if devices_path.is_some() {
        let mut all = netlist_switch_channels;
        all.extend(device_switch_channels);
        all
    } else {
        device_switch_channels
    };
    let diode_curves = if devices_path.is_some() {
        let mut all = netlist_diode_curves;
        all.extend(device_diode_curves);
        all
    } else {
        device_diode_curves
    };
    let general_mna::System {
        ideal_diodes: diodes,
        ideal_switches,
        gates,
        blocks,
        shared_r_on,
        ..
    } = general_mna::build_system(&devices_for_build, dialect)
        .map_err(|e| format!("parsing device/block declarations: {e}"))?;

    // Checks that can be made from the declaration alone, before a transient that may take
    // minutes. Warnings, not errors: a deliberately disabled leg is a legitimate deck, and a
    // run that refused to start would be the wrong answer to a gate someone pinned at zero on
    // purpose. See `static_checks`'s own module doc for why nothing here guesses.
    for warning in static_checks::dead_gates(&gates, &blocks) {
        eprintln!("{warning}");
    }

    // A `kind=sig2phys` converter has no terminals: it is consumed *by name* (a V/I source's own
    // value field, or an ideal switch's `gate=`/`ctrl=`), never by a wire. Catching a converter
    // wired into the netlist as an ordinary node here -- rather than only inside
    // `simulate_transient_with_blocks_streamed` -- is what makes `--mode dc` fail on it too: that
    // path never enters the block-graph engine at all (`solve_dc` takes no blocks), so it would
    // otherwise still report the silent `V(<name>) = 0` this check exists to prevent.
    let statements = general_mna::parse_and_flatten(&netlist, dialect)
        .map_err(|e| format!("parsing netlist: {e}"))?;
    if let Err(e) = dae_runtime::reject_sig2phys_wired_into_circuit(&statements, &blocks) {
        // Spelled out rather than left as the bare `{e:?}` dump every other DaeError gets here:
        // the whole point of this check is that the *silent* failure was indistinguishable from a
        // legitimately-zero node, so the error has to say what to write instead. The `{e:?}`
        // prefix is kept so the variant name is still greppable in stderr, like every other one.
        return Err(match &e {
            DaeError::Sig2PhysUsedAsCircuitNode {
                block,
                element,
                node,
            } => format!(
                "{e:?}: element '{element}' wires node '{node}', but '{block}' is a \
                 kind=sig2phys converter, not a device with terminals -- it has no pins and \
                 stamps nothing into the circuit, so that node would be undriven and silently \
                 solve to V({node}) = 0. A converter is consumed by name, never by a wire: name \
                 it in an independent source's own value field (e.g. `V1 a 0 {block}`) or in an \
                 ideal switch's gate=/ctrl= field."
            ),
            other => format!("{other:?}"),
        });
    }
    // The event-file overrides, resolved once. `None` keeps the `<netlist stem>.events.*`
    // convention a person running the binary by hand expects.
    let event_paths = rules::EventPaths {
        jsonl: events_jsonl.as_deref().map(PathBuf::from),
        text: events_text.as_deref().map(PathBuf::from),
    };

    let (waveform, plot_name) = if mode == "dc" {
        let point = if ideal_switches.is_empty() {
            solve_dc(&netlist, dialect, &diodes).map_err(|e| dae_message(&e))?
        } else {
            // Every gate is block-driven (gate=block) -- a DC operating point has no notion of
            // a block's time-stepped state, so any ideal switch at all makes --mode dc unsupported.
            let name = ideal_switches
                .keys()
                .next()
                .expect("ideal_switches is non-empty here");
            return Err(format!(
                "device '{name}': every gate is block-driven (gate=block), which needs \
                 --mode transient, not 'dc' (a DC operating point has no notion of a block's \
                 time-stepped state)"
            ));
        };
        let mut headers = vec!["t".to_string()];
        headers.extend(point.unknowns.iter().cloned());
        let mut row = vec![0.0];
        row.extend(point.x.iter().copied());
        (
            Waveform {
                headers,
                rows: vec![row],
            },
            "DC transfer characteristic",
        )
    } else if mode == "transient" {
        // Every transient run streams, whatever is in the deck.
        //
        // There used to be a second, buffered path here, taken when a deck had no ideal
        // switches, no blocks, no checkpointing and no declared rules: it called
        // `simulate_transient`, collected the whole trace, and built a `Waveform` for the
        // shared serialization below. The condition grew by subtraction -- blocks, then
        // switches, then checkpointing, then rules -- because each feature turned out to be
        // wired into the streamed path only, and each time the comment here recorded that the
        // streamed path handled the plain case identically anyway.
        //
        // It had to go because `Waveform` holds `Vec<Vec<f64>>`: the entire run, plus one heap
        // allocation per row. At four million rows and seventy signals that is about 2.2 GB of
        // samples and four million allocations, and ten grid periods is ten times that, so the
        // process died before writing anything. The limit was invisible because it depended on
        // the deck's *features* rather than its size -- every power-electronics deck has
        // switches and already streamed -- so what could not be run was a long sweep of a plain
        // linear deck: a thermal network, a finely resolved filter. Nothing in the flags hinted
        // that the longest run you could do depended on whether your circuit happened to
        // contain a switch (#106).
        //
        // `simulate_transient_with_blocks_streamed` tolerates empty `ideal_switches`, `gates`
        // and `blocks` equally well (see its own doc comment), so the plain case is now the
        // streamed path with nothing for a block-graph step to resolve. Peak memory is one row.
        let checkpointing = checkpoint_out.is_some() || resume.is_some();
        if checkpoint_every.is_some() && checkpoint_out.is_none() {
            return Err("--checkpoint-every needs --checkpoint-out <file>".to_string());
        }
        if checkpointing && mode != "transient" {
            return Err("--checkpoint-out/--resume only apply to --mode transient".to_string());
        }
        // Streams CSV/raw/samples output directly and writes its own measurement log, rather
        // than building the shared (waveform, plot_name) tuple this match falls through to
        // below -- see run_transient_streamed's own doc comment for why. Returns from `run`
        // immediately, skipping the shared print_csv/write_raw_file/write_measurements_log
        // block entirely, since it already did the equivalent work incrementally.
        return run_transient_streamed(
            &netlist,
            dialect,
            &diodes,
            &ideal_switches,
            &gates,
            &blocks,
            shared_r_on,
            t_final,
            step,
            max_steps.unwrap_or(ADAPTIVE_STEP_HARD_CAP),
            output_format,
            out_every,
            raw_out_path.as_deref(),
            samples_out_path.as_deref(),
            netlist_path,
            &measurements,
            declared_rules,
            &declared_ports,
            &switch_channels,
            &diode_curves,
            &event_paths,
            fail_on,
            CheckpointArgs {
                out: checkpoint_out.as_deref().map(Path::new),
                every: checkpoint_every,
                resume: resume.as_deref().map(Path::new),
            },
        );
    } else {
        return Err(format!(
            "unknown --mode '{mode}' (expected 'dc' or 'transient')"
        ));
    };

    // `--out-every` thins the *output* only, so the decimated copy is what gets serialized while
    // `waveform` -- every resolved point -- is what the measurements below are evaluated against.
    // Decimating a measurement's input would change its result rather than its size, which is a
    // different feature and a worse one.
    let out_waveform = decimate(&waveform, out_every);
    match output_format {
        OutputFormat::Csv => print_csv(&out_waveform),
        OutputFormat::Raw => {
            let out_path = raw_out_path.expect("raw_out_path is Some when output_format is Raw");
            write_raw_file(&out_path, &out_waveform, plot_name, netlist_path)?;
        }
        OutputFormat::Samples => {
            let out_path =
                samples_out_path.expect("samples_out_path is Some when output_format is Samples");
            // Every column after `t` here is an MNA unknown: this path is `--mode dc`, which
            // runs no block graph at all.
            let circuit_unknowns = out_waveform.headers.len().saturating_sub(1);
            write_samples_file(
                &out_path,
                &out_waveform,
                circuit_unknowns,
                &[],
                signal_meta::Analysis::Dc,
            )?;
        }
    }

    // Measurements write to a `<netlist stem>.log` file, never stdout or stderr, matching real
    // SPICE tools' own `.measure`/`.MEASURE` convention (ngspice/Xyce both write measurement
    // results to a log file alongside the netlist, not to the console) -- see
    // `book/user-guide/src/measurements.md`'s "Where results are written" section. This keeps
    // stdout's own machine-readability (a plain CSV under `--format csv`, or nothing at all
    // under `--format raw`, which already writes to a file) unconditionally intact: a
    // `kind=measure`-free netlist produces byte-for-byte the same stdout as before this feature
    // existed, and even a netlist that does use it never interleaves "name = value" lines into
    // the CSV a downstream tool parses, and never mixes them with unrelated stderr diagnostics
    // either.
    if !measurements.is_empty() {
        write_measurements_log(&measurements, &waveform, &default_log_path(netlist_path))?;
    }

    Ok(exit::OK)
}

/// `<netlist stem>.log` next to the input file -- where `kind=measure` results are written,
/// matching real SPICE tools' own measurement-log convention. Always derived from the netlist
/// path (no `--out`-style override exists for this, unlike `--format raw`'s own output path --
/// measurements are a secondary artifact of a run, not its primary requested output).
fn default_log_path(netlist_path: &str) -> PathBuf {
    Path::new(netlist_path).with_extension("log")
}

/// `<netlist stem>.raw` next to the input file — the default `--out` destination for
/// `--format raw` when `--out` isn't given, so `general-simulator some.cir --mode transient
/// --format raw` just works without also requiring `--out some.raw`.
fn default_raw_path(netlist_path: &str) -> PathBuf {
    let path = Path::new(netlist_path);
    path.with_extension("raw")
}

/// Keeps every `stride`-th row, starting at the first. `stride == 1` returns the waveform
/// unchanged, which is the default and every pre-`--out-every` caller's behavior.
///
/// Borrowing would be tidier than cloning, but the rows are `Vec<f64>` inside a `Waveform` the
/// raw writer takes by reference, and a run large enough for this flag to matter is one whose
/// decimated copy is by construction small.
fn decimate(waveform: &Waveform, stride: usize) -> Waveform {
    if stride <= 1 {
        return waveform.clone();
    }
    Waveform {
        headers: waveform.headers.clone(),
        rows: waveform.rows.iter().step_by(stride).cloned().collect(),
    }
}

/// Writes a finished waveform to a `.gsim_samples` container.
///
/// Only the buffered path reaches this -- a deck with no switches, no blocks
/// and no rules, whose whole trace is already in memory. Every run large
/// enough for this format to be the reason you chose it takes the streamed
/// path instead and never materialises a waveform at all, which is the
/// point: see `run_transient_streamed`.
fn write_samples_file(
    out_path: &Path,
    waveform: &Waveform,
    circuit_unknowns: usize,
    blocks: &[BlockInstance],
    analysis: signal_meta::Analysis,
) -> Result<(), String> {
    let file = fs::File::create(out_path)
        .map_err(|e| format!("creating samples output file {}: {e}", out_path.display()))?;
    let (sweep, signals) =
        signal_meta::vector_meta(&waveform.headers, circuit_unknowns, blocks, analysis);
    let mut writer = gsim_samples::Writer::new(BufWriter::new(file), sweep, signals)
        .map_err(|e| format!("writing {}: {e}", out_path.display()))?;
    for row in &waveform.rows {
        writer
            .push(row[0], &row[1..])
            .map_err(|e| format!("writing {}: {e}", out_path.display()))?;
    }
    writer
        .finish()
        .map_err(|e| format!("writing {}: {e}", out_path.display()))?;
    Ok(())
}

fn write_raw_file(
    out_path: &Path,
    waveform: &Waveform,
    plot_name: &'static str,
    netlist_path: &str,
) -> Result<(), String> {
    let file = fs::File::create(out_path)
        .map_err(|e| format!("creating raw output file {}: {e}", out_path.display()))?;
    let mut writer = BufWriter::new(file);
    // `--mode dc` is the only caller that still materialises a whole waveform, and it has no
    // blocks, so every column after `t` is an MNA unknown.
    let quantities = signal_meta::quantities(
        &waveform.headers,
        waveform.headers.len().saturating_sub(1),
        &[],
    );
    let rendered = raw_format::Waveform {
        names: &waveform.headers,
        quantities: &quantities,
        rows: &waveform.rows,
        plot_name,
        title: netlist_path,
    };
    raw_format::write_raw(&mut writer, &rendered)
        .map_err(|e| format!("writing raw output file {}: {e}", out_path.display()))
}

/// `--mode dc` has no notion of a block's time-stepped state (no transient loop runs at all),
/// but every gate is now block-driven (`gate=block ctrl=<name>`, enforced by `general_mna::
/// build_system` itself) — so a `.op`-style DC operating point with any ideal switch present has
/// nothing to resolve its gate from and is unconditionally unsupported, not just for the cases
/// that used to need a block. `--mode transient` with at least one ideal switch *or* at least one
/// block declared (an ideal switch-free block-graph study is just as legitimate as a block-free
/// ideal switch circuit — see this file's own `main` for the exact condition): resolves every gate
/// (always block-driven) via [`dae_runtime::simulate_transient_with_blocks_streamed`], which
/// tolerates an empty `ideal_switches`/`gates` map or an empty `blocks` slice equally well — see
/// this file's module doc comment for why there's no separate mode for the block-driven case.
///
/// Genuinely streams: CSV rows go straight to stdout and raw-format rows go straight to a temp
/// file, both as each step is produced, never accumulated as a `Vec` covering the whole run (the
/// way `run_transient_with_ideal_switches` — this function's predecessor, removed — used to).
/// This is the fix for a real incident: a single run of a large, numerically stiff circuit drove
/// this project's own development machine to ~11GB RSS / 24GB swap before being killed, entirely
/// because of that old function's full-`Vec` buffering (see an internal PI/PR
/// modulator-comparison experiment for the incident write-up).
///
/// The one thing that still needs *some* history kept in memory is `kind=measure` — a
/// measurement needs the whole time series of whichever signal(s) it names, not just the latest
/// row — but only for the specific columns some measurement spec actually references
/// ([`measure::referenced_signals`]), never the full circuit's worth of columns; a run with
/// measurements on 2 signals out of 70 columns keeps roughly 2/70th of the memory a full-`Vec`
/// approach would, regardless of the netlist's real column count.
///
/// A binary rawfile's own `No. Points:` header field is read upfront by every reader this format
/// was validated against, so it can't be written until the real point count is known — hence the
/// temp-file-then-copy dance below: row data streams straight to `<out>.raw.tmp`, and once the
/// real count is known (the run finished), the actual header (with that count) is written to the
/// real output path immediately followed by a plain byte-for-byte copy of the temp file, which is
/// then deleted. Memory cost is the same either way (streamed regardless), but disk cost is
/// briefly ~2x the final `.raw` file's size for the temp copy — an acceptable trade against never
/// holding row data in memory, and no worse than what a network/pipe-based writer with the same
/// "point count must come first" constraint would also have to do.
#[allow(clippy::too_many_arguments)]
fn run_transient_streamed(
    netlist: &str,
    dialect: Dialect,
    diodes: &BTreeMap<String, IdealDiode>,
    ideal_switches: &BTreeMap<String, IdealSwitch>,
    gates: &BTreeMap<String, GateBinding>,
    blocks: &[BlockInstance],
    shared_r_on: f64,
    t_final: f64,
    step: TimeStep,
    max_steps: usize,
    output_format: OutputFormat,
    out_every: usize,
    raw_out_path: Option<&Path>,
    samples_out_path: Option<&Path>,
    netlist_path: &str,
    measurements: &[measure::MeasureSpec],
    declared_rules: Vec<gsim_rules::Rule>,
    declared_ports: &BTreeMap<String, rules::Port>,
    switch_channels: &BTreeMap<String, rules::SwitchChannel>,
    diode_curves: &BTreeMap<String, rules::DiodeCurve>,
    event_paths: &rules::EventPaths,
    // The lowest severity that makes this process exit `RULE_ERROR` (`--fail-on`).
    fail_on: gsim_rules::Severity,
    checkpoints: CheckpointArgs<'_>,
) -> Result<u8, String> {
    // Which solver drives the output below. The block-graph solver handles an empty
    // `ideal_switches`/`gates`/`blocks` perfectly well, but it is not free to do so: measured on
    // a two-million-step plain RC deck with the output thinned a million-fold, so that only the
    // solve is timed, it took 12.69 s against the plain solver's 5.56 s. That is 2.3x per step
    // for machinery with nothing to resolve, which is too much to charge a deck that has no
    // blocks in it.
    //
    // So the choice of solver stays, and what went is the buffering. Both now drive the same
    // `on_step` closure -- the writers, `--out-every`, the measurement capture -- so there is
    // one output pipeline and the solver is an implementation detail of it. Rules and
    // checkpointing are wired into the block-graph path only, which is why their absence is
    // part of the condition.
    let plain = ideal_switches.is_empty()
        && blocks.is_empty()
        && declared_rules.is_empty()
        && checkpoints.out.is_none()
        && checkpoints.resume.is_none();

    let resume_from = match checkpoints.resume {
        Some(path) => Some(Checkpoint::read_from(path).map_err(|e| dae_message(&e))?),
        None => None,
    };
    // Written on every periodic hand-out and again at the end -- the same file, atomically
    // replaced (see `Checkpoint::write_to`), so what is on disk is always the latest complete one.
    let mut write_checkpoint = |c: &Checkpoint| -> Result<(), DaeError> {
        match checkpoints.out {
            Some(path) => c.write_to(path),
            None => Ok(()),
        }
    };
    // A cscript, coordinate-transform, pmsm, pwm, pspwm or filewave block registers extra named
    // outputs beyond its own block name (see block_graph::evaluate_blocks) -- list those too, so
    // they show up as their own CSV columns instead of only being reachable via Signal::Block
    // from another declared block.
    //
    // `filewave` is the one where leaving it out is not merely inconvenient: the whole reason to
    // read a capture is to compare a run against it, and a `kind=measure type=error ref=` can
    // only name a column. A second channel that evaluated but never reached the waveform was
    // reachable by nothing at all. Doesn't need a single row resolved -- purely a function of `blocks`
    // itself -- so, unlike the header's own vector-arity expansion below, this part is known
    // upfront.
    let mut block_names: Vec<String> = blocks.iter().map(|b| b.name.clone()).collect();
    for block in blocks {
        match &block.kind {
            BlockKind::CScript { output_names, .. }
            | BlockKind::PyBlock { output_names, .. }
            | BlockKind::PyFunction { output_names, .. }
            | BlockKind::OctFunc { output_names, .. }
            | BlockKind::OctBlock { output_names, .. }
            | BlockKind::CoordinateTransform { output_names, .. }
            | BlockKind::Pmsm { output_names, .. }
            | BlockKind::Pwm { output_names, .. }
            | BlockKind::PhaseShiftPwm { output_names, .. }
            | BlockKind::FileWave { output_names, .. } => {
                block_names.extend(output_names.iter().skip(1).cloned());
            }
            _ => {}
        }
    }

    let referenced = measure::referenced_signals(measurements);
    // (name -> (ts, vs)), one entry per column any measurement spec actually names -- the only
    // per-row history this function keeps beyond the current row, and only for these columns.
    let mut measured: BTreeMap<String, (Vec<f64>, Vec<f64>)> = referenced
        .iter()
        .map(|n| (n.clone(), (Vec::new(), Vec::new())))
        .collect();
    // Position of each *found* referenced name within the header built from the first row -- a
    // name that never matches any real column is simply absent here (and from `measured`'s
    // captured data), which is exactly the "unknown signal" case `evaluate_one`/`samples_of`
    // already report as an error today; this function doesn't special-case it.
    let mut referenced_index: Vec<(String, usize)> = Vec::new();
    let mut headers: Option<Vec<String>> = None;
    // What each of those columns is, resolved alongside them in the same first-row branch --
    // see `signal_meta`.
    let mut quantities: Option<Vec<signal_meta::Quantity>> = None;

    let stdout = io::stdout();
    let mut csv_writer =
        (output_format == OutputFormat::Csv).then(|| BufWriter::new(stdout.lock()));

    // No temporary file and no second pass, unlike the rawfile above: a
    // `.gsim_samples` footer is written last by design, precisely so that a
    // run can be streamed into it without knowing its length in advance.
    let mut samples_writer: Option<gsim_samples::Writer<BufWriter<fs::File>>> = None;

    let raw_tmp_path = raw_out_path.map(|p| p.with_extension("raw.tmp"));
    let mut raw_tmp_writer = match &raw_tmp_path {
        Some(p) => Some(BufWriter::new(fs::File::create(p).map_err(|e| {
            format!("creating temporary raw output {}: {e}", p.display())
        })?)),
        None => None,
    };
    let mut n_points: usize = 0;
    let mut n_resolved: usize = 0;

    // Rules, if the netlist declared any. The two files are created only when it did, the same
    // way `kind=measure` only ever creates its `.log` for a netlist that asks for one.
    let mut engine = gsim_rules::RuleEngine::new(declared_rules, false);
    let mut reporter = if engine.is_empty() {
        None
    } else {
        Some(rules::Reporter::new(
            Path::new(netlist_path),
            gsim_rules::Severity::Warning,
            event_paths,
        )?)
    };
    let mut steps_seen: usize = 0;
    // The unknown that went non-finite first, once one has. Held outside the step closure so
    // the run can report it after the loop, beside a rule stop.
    let mut non_finite: Option<(String, usize, f64)> = None;

    let mut on_step = |t: f64,
                       point: dae_runtime::OperatingPoint,
                       outputs: BTreeMap<String, dae_runtime::SignalValue>|
     -> Result<(), DaeError> {
        if headers.is_none() {
            // A name whose value is a SignalValue::Vector expands into one CSV column per
            // element (NAME[0], NAME[1], ...) rather than one column holding the whole vector
            // -- arity is fixed for the whole run once a block declares it (see
            // book/dev-guide/src/vector-signals.md), so deriving each name's own column count
            // from the *first* row's resolved shape is exactly as valid as a separate static
            // analysis would be.
            let mut h = vec!["t".to_string()];
            h.extend(point.unknowns.iter().cloned());
            if !block_names.is_empty() {
                h.extend(block_names.iter().flat_map(|name| {
                    match outputs.get(name) {
                        Some(dae_runtime::SignalValue::Vector(v)) => (0..v.len())
                            .map(|i| format!("{name}[{i}]"))
                            .collect::<Vec<_>>(),
                        _ => vec![name.clone()],
                    }
                }));
            }
            referenced_index = referenced
                .iter()
                .filter_map(|name| {
                    h.iter()
                        .position(|c| c == name)
                        .map(|idx| (name.clone(), idx))
                })
                .collect();
            if let Some(w) = &mut csv_writer {
                writeln!(w, "{}", h.join(",")).map_err(|e| DaeError::Io(e.to_string()))?;
            }
            // Here rather than before the loop because the columns are
            // not known until the first row resolves: a vector-valued
            // block signal expands into one column per element, and its
            // arity comes from that row (see the comment above).
            quantities = Some(signal_meta::quantities(&h, point.unknowns.len(), blocks));
            if let Some(path) = samples_out_path {
                let file = fs::File::create(path)
                    .map_err(|e| DaeError::Io(format!("creating {}: {e}", path.display())))?;
                let (sweep, signals) = signal_meta::vector_meta(
                    &h,
                    point.unknowns.len(),
                    blocks,
                    signal_meta::Analysis::Transient,
                );
                samples_writer = Some(
                    gsim_samples::Writer::new(BufWriter::new(file), sweep, signals)
                        .map_err(|e| DaeError::Io(e.to_string()))?,
                );
            }
            headers = Some(h);
        }

        let mut row = vec![t];
        row.extend(point.x.iter().copied());
        if !block_names.is_empty() {
            row.extend(block_names.iter().flat_map(|name| match outputs.get(name) {
                Some(dae_runtime::SignalValue::Scalar(x)) => vec![*x],
                Some(dae_runtime::SignalValue::Vector(v)) => v.clone(),
                None => vec![f64::NAN],
            }));
        }

        for (name, idx) in &referenced_index {
            if let Some((ts, vs)) = measured.get_mut(name) {
                ts.push(t);
                vs.push(row[*idx]);
            }
        }

        // Before the rules, because every one of them compares a value against a limit and
        // every such comparison against a `NaN` is false: a run that has gone non-finite
        // would otherwise report nothing, summarise as `passed` and exit 0. Checked on
        // every step whether or not any rule was declared -- see `finite`'s module doc for
        // why this one is not opt-in.
        if non_finite.is_none() {
            let broken = finite::first_non_finite(&point)
                .map(|(name, value)| (name.to_string(), value))
                .or_else(|| finite::first_non_finite_output(&outputs));
            if let Some((name, value)) = broken {
                let record = finite::event(&name, value, steps_seen, t);
                eprintln!("{}", record.console_line());
                if let Some(reporter) = &mut reporter {
                    reporter.record(&record).map_err(DaeError::Io)?;
                }
                non_finite = Some((name, steps_seen, t));
            }
        }

        // Rules see every step, not every *written* point. `--out-every` thins the
        // waveform file, and an edge or a peak falling between two written points is
        // exactly what a rule exists to catch -- so this sits above the thinning return
        // below, not beside the writers.
        if let Some(reporter) = &mut reporter {
            let gate_states = rules::gate_states(gates, &outputs);
            let sample = rules::StepSample {
                point: &point,
                outputs: &outputs,
                gates: &gate_states,
                ports: declared_ports,
                switches: switch_channels,
                diodes: diode_curves,
            };
            for event in engine.observe(steps_seen, t, &sample) {
                reporter.record(&event).map_err(DaeError::Io)?;
            }
            steps_seen += 1;
        }
        // Asked for here, acted on at the end of this closure: the step that tripped the
        // rule has already been solved, its state is what the checkpoint will carry, and a
        // resume continues from *after* it -- so its row belongs in the waveform. Returning
        // here instead would leave a one-row hole exactly at the interesting moment.
        let stop_now = engine.stop_requested().is_some() || non_finite.is_some();

        // `--out-every N` thins what is *written*, never what is computed: every step is
        // still taken, every measurement above still sees every point, and the solution is
        // bit-identical to the same run without the flag. Only serialization is skipped.
        //
        // The counter is over resolved points rather than time, so a decimated adaptive run
        // stays readable (uneven in time, but that is what adaptive stepping already is).
        // Point 0 is always emitted; the final point is emitted only if its index happens to
        // land on the stride, which keeps the rule to one sentence -- "every Nth point" --
        // rather than a special case a reader has to remember.
        let emit = n_resolved.is_multiple_of(out_every);
        n_resolved += 1;
        if !emit {
            // A stop landing on a thinned-out point still stops; it just has no row.
            return if stop_now {
                Err(DaeError::StopRequested)
            } else {
                Ok(())
            };
        }

        if let Some(w) = &mut csv_writer {
            let values: Vec<String> = row.iter().map(|v| v.to_string()).collect();
            writeln!(w, "{}", values.join(",")).map_err(|e| DaeError::Io(e.to_string()))?;
        }
        if let Some(w) = &mut raw_tmp_writer {
            raw_format::write_row(w, &row, row.len()).map_err(|e| DaeError::Io(e.to_string()))?;
        }
        if let Some(w) = &mut samples_writer {
            w.push(row[0], &row[1..])
                .map_err(|e| DaeError::Io(e.to_string()))?;
        }
        n_points += 1;
        // Cooperative: the loop finishes this step, leaves normally and still builds the
        // final checkpoint, so the run can be resumed from exactly here.
        if stop_now {
            return Err(DaeError::StopRequested);
        }
        Ok(())
    };

    let final_checkpoint = if plain {
        // The plain solver hands its step back by reference and has no block outputs, so the
        // closure sees an empty map -- exactly what it would have seen from the block-graph
        // solver on a deck with no blocks. One clone per step, where the old buffered path
        // cloned every step and then kept it.
        simulate_transient_streamed(netlist, dialect, diodes, None, t_final, step, |t, point| {
            on_step(t, point.clone(), BTreeMap::new())
        })
        .map(|()| None)
    } else {
        simulate_transient_with_blocks_checkpointed(
            netlist,
            dialect,
            diodes,
            ideal_switches,
            blocks,
            gates,
            shared_r_on,
            None,
            t_final,
            step,
            max_steps,
            CheckpointControl {
                resume: resume_from.as_ref(),
                every: checkpoints.every,
                on_checkpoint: checkpoints
                    .every
                    .map(|_| &mut write_checkpoint as &mut CheckpointSink<'_>),
                final_snapshot: checkpoints.out.is_some(),
            },
            &mut on_step,
        )
    }
    .map_err(|e| dae_message(&e))?;

    // Close every rule out before the files are flushed, so a violation still in progress when
    // the run ended is reported as a truncated leave rather than vanishing, and the summary
    // accounts for its time.
    let stopped_by = engine.stop_requested().map(str::to_string);
    if let Some(reporter) = &mut reporter {
        let (tail, summaries) = engine.finish(steps_seen);
        for event in &tail {
            reporter.record(event)?;
        }
        reporter.finish(&summaries)?;
    }

    if let (Some(path), Some(final_checkpoint)) = (checkpoints.out, final_checkpoint) {
        final_checkpoint
            .write_to(path)
            .map_err(|e| dae_message(&e))?;
    }

    if let Some(w) = &mut csv_writer {
        w.flush()
            .map_err(|e| format!("writing CSV to stdout: {e}"))?;
    }

    if let Some(writer) = samples_writer {
        let path = samples_out_path.expect("a writer exists only when a path was given");
        writer
            .finish()
            .map_err(|e| format!("writing {}: {e}", path.display()))?;
    }

    if let Some(tmp_path) = &raw_tmp_path {
        // Drop (flush + close) the temp file before reopening it for the copy below.
        drop(raw_tmp_writer);
        let out_path = raw_out_path.expect("raw_out_path is Some when raw_tmp_path is Some");
        let final_headers = headers.clone().unwrap_or_else(|| vec!["t".to_string()]);
        let final_quantities = quantities
            .clone()
            .unwrap_or_else(|| vec![signal_meta::Quantity::Time]);
        let file = fs::File::create(out_path)
            .map_err(|e| format!("creating raw output file {}: {e}", out_path.display()))?;
        let mut writer = BufWriter::new(file);
        raw_format::write_header(
            &mut writer,
            &final_headers,
            &final_quantities,
            n_points,
            "Transient Analysis",
            netlist_path,
        )
        .map_err(|e| format!("writing raw output file {}: {e}", out_path.display()))?;
        if n_points > 0 {
            let mut tmp_file = fs::File::open(tmp_path)
                .map_err(|e| format!("reading temporary raw output {}: {e}", tmp_path.display()))?;
            io::copy(&mut tmp_file, &mut writer).map_err(|e| {
                format!(
                    "copying temporary raw output into {}: {e}",
                    out_path.display()
                )
            })?;
        }
        writer
            .flush()
            .map_err(|e| format!("writing raw output file {}: {e}", out_path.display()))?;
        // Best-effort cleanup -- a failure here doesn't invalidate the real output file already
        // written above, so it's not worth failing the whole run over.
        let _ = fs::remove_file(tmp_path);
    }

    if !measurements.is_empty() {
        // The "shadow" waveform: `t` plus only the columns some measurement spec actually
        // referenced, built from `measured`'s own small per-column history -- this is the whole
        // point of tracking `referenced`/`measured` above rather than the full trace.
        let names_in_order: Vec<String> = referenced_index.iter().map(|(n, _)| n.clone()).collect();
        let series: Vec<&(Vec<f64>, Vec<f64>)> =
            names_in_order.iter().map(|n| &measured[n]).collect();
        let n_rows = series.first().map_or(0, |(ts, _)| ts.len());
        let mut shadow_headers = vec!["t".to_string()];
        shadow_headers.extend(names_in_order);
        let mut shadow_rows = Vec::with_capacity(n_rows);
        for i in 0..n_rows {
            let mut row = Vec::with_capacity(shadow_headers.len());
            row.push(series[0].0[i]);
            row.extend(series.iter().map(|(_, vs)| vs[i]));
            shadow_rows.push(row);
        }
        let shadow = Waveform {
            headers: shadow_headers,
            rows: shadow_rows,
        };
        write_measurements_log(measurements, &shadow, &default_log_path(netlist_path))?;
    }

    // The run itself succeeded in every case that reaches here, including a rule stop: the
    // waveform, the event log and the checkpoint are all written and readable. The code says
    // what the rules found.
    // Checked before a rule stop: when both happened on the same step, the non-finite state
    // is the one that explains the run, and a rule that fired on a `NaN`-adjacent value is a
    // consequence rather than a finding.
    if let Some((name, step, when)) = non_finite {
        eprintln!(
            "abandoned at step {step} (t = {when:e} s): '{name}' is not a finite number. \
             Nothing in the waveform after this point means anything."
        );
        return Ok(exit::NOT_FINITE);
    }
    if let Some(name) = stopped_by {
        eprintln!("stopped by rule '{name}'");
        return Ok(exit::STOPPED_BY_RULE);
    }
    match reporter.as_ref().and_then(|r| r.worst) {
        Some(severity) if severity >= fail_on => Ok(exit::RULE_ERROR),
        _ => Ok(exit::OK),
    }
}

/// Prints `waveform` as CSV to stdout, exactly reproducing this crate's original (pre-`--format`)
/// output byte-for-byte: `t,V(node1),V(node2),...,<block names>` header, then one comma-joined
/// row per point, each value rendered with `f64`'s own `Display` (the same as `to_string()`
/// always used here) rather than a fixed format -- this is the historical, still-default
/// behavior every existing test and worked example depends on.
fn print_csv(waveform: &Waveform) {
    // An empty trace (no resolved points at all) prints nothing, not even the header -- matching
    // this crate's original behavior of only calling `print_header` once `trace.first()` proved
    // there was at least one point to describe a header for.
    if waveform.rows.is_empty() {
        return;
    }
    println!("{}", waveform.headers.join(","));
    for row in &waveform.rows {
        let values: Vec<String> = row.iter().map(|v| v.to_string()).collect();
        println!("{}", values.join(","));
    }
}

/// Evaluates every collected `kind=measure` spec against the resolved `waveform` and writes each
/// result to `log_path` as `name = value`, ngspice's own `.measure` printed style (see the
/// ngspice manual's `.measure` section: `tdiff = 1.000000e-003 targ= ... trig= ...`), but to a
/// log FILE rather than the console — matching real SPICE tools' own convention of writing
/// measurement results to a `.log` file alongside the netlist, not interleaving them with
/// console/stderr diagnostics. A DC run (`waveform.rows.len() <= 1`) or a `--mode dc` netlist
/// can't sensibly host a measurement that needs a real window/crossing search over a trace, so a
/// per-measurement error there (from `gs_waveform_measurements`'s own `EmptyWindow`/
/// `EmptySeries`) is written the same as any other per-measurement evaluation failure, not
/// specially detected — the failure message is already precise about why. Only called when
/// `measurements` is non-empty (see the caller), so a `kind=measure`-free netlist never creates
/// a `.log` file at all.
fn write_measurements_log(
    measurements: &[measure::MeasureSpec],
    waveform: &Waveform,
    log_path: &Path,
) -> Result<(), String> {
    let file = fs::File::create(log_path)
        .map_err(|e| format!("creating measurement log {}: {e}", log_path.display()))?;
    let mut writer = BufWriter::new(file);
    let results = measure::evaluate_all(measurements, &waveform.headers, &waveform.rows);
    for (name, result) in results {
        match result {
            Ok(values) => {
                for (label, value) in values {
                    writeln!(writer, "{label} = {value}").map_err(|e| {
                        format!("writing measurement log {}: {e}", log_path.display())
                    })?;
                }
            }
            Err(e) => writeln!(writer, "measurement '{name}' failed: {e}")
                .map_err(|e| format!("writing measurement log {}: {e}", log_path.display()))?,
        }
    }
    writer
        .flush()
        .map_err(|e| format!("writing measurement log {}: {e}", log_path.display()))
}

/// Parses a `<signal>` field value: `prev:<block>` for a named block's own output from the
/// *previous* step (`0.0` before the first step) — needed to close a loop around a block itself
/// (a controller regulating a `kind=pmsm`'s own `id`/`iq` outputs, or a PLL's angle estimate
/// feeding the very `kind=park` block that produced its own error signal), where a same-step
/// reference would be a genuine algebraic loop. Anything else is another block's name (this
/// same step's output) — including a `kind=phys2sig` block, the *only* legal way to read a circuit
/// quantity into the signal domain (there is deliberately no `meas:`-style inline shortcut
/// anymore; see `kind=phys2sig`'s own doc section above). A stray `meas:<node>` left over from
/// before this convention was enforced is simply treated as an ordinary (and therefore unknown)
/// block name, surfacing as a clear `UnknownBlockInput` error rather than silently reading the
/// circuit.
/// The three checkpoint flags, passed together -- see the module doc comment's own
/// "Checkpoint and resume" section.
struct CheckpointArgs<'a> {
    out: Option<&'a Path>,
    every: Option<f64>,
    resume: Option<&'a Path>,
}

/// `--version` output: the program name and version, one line, on stdout.
///
/// "general-simulator", not `CARGO_PKG_NAME` -- the crate is `general-simulator-cli` and
/// the binary it produces is `general-simulator`. The name a user types, and the name
/// anything looking for the binary searches for, is the latter.
fn version_line() -> String {
    format!("general-simulator {}", env!("CARGO_PKG_VERSION"))
}

/// `--help` output.
///
/// Deliberately not [`usage`]. That one is the long prose shown when an argument is
/// *wrong*, and explains the reasoning behind each flag at length -- useful once, and a
/// wall of text to land on after a typo. This is the scannable version: the synopsis, the
/// flags grouped by what they decide, a default for every one that has one, and three
/// invocations worth copying. The reasoning lives in the book, which the last line names.
fn help() -> String {
    format!(
        "\
{version}
A piecewise-linear circuit and block-diagram simulator. Device segments are resolved as a
linear complementarity problem rather than by Newton-Raphson, so there is no voltage limiting.

USAGE
    general-simulator <netlist> [OPTIONS]

    <netlist>                 The .cir deck to run. Required, and positional: it comes
                              first. Device and block declarations may sit in the same
                              file or in a separate --devices file.

ANALYSIS
    --mode dc|transient       `dc` solves a single operating point; `transient` runs the
                              resolved timestep loop. [default: dc]
    --tfinal T                End time in seconds, under --mode transient. [default: 1.0]

TIME STEPPING                 Fixed or adaptive -- these two groups are mutually exclusive.
    --dt DT                   Fix the step size. Deterministic and exactly reproducible;
                              the right choice while a deck is being developed.
    --dt-max T                Adaptive bounds, used when --dt is omitted.
    --dt-min T
    --dt-init T
    --reltol R                Adaptive local-truncation-error tolerances. --abstol has to
    --abstol A                match your own circuit's signal magnitudes: a value carried
                              over from a smaller circuit drives the step size into the
                              ground.
    --max-steps N             Cap on accepted adaptive steps, after which the run aborts
                              rather than growing its in-memory trace without limit.
                              Adaptive only. [default: 10000000]

OUTPUT
    --format csv|raw|samples  `csv` prints to stdout; `raw` writes a binary SPICE rawfile,
                              which ngspice and PySpice read; `samples` writes a
                              .gsim_samples container. [default: csv]
                              Pick `samples` for a long run you intend to look at. The
                              other two are row-major, so plotting five of seventy signals
                              means reading and parsing all seventy, and showing one
                              millisecond out of two hundred means scanning from the start.
                              `samples` stores each signal separately with a min/max
                              pyramid over it, so a viewer reads a screen's worth of points
                              whatever the run's length.
    --out <path>              Destination for --format raw and --format samples. Ignored
                              for csv, which always goes to stdout -- redirect it.
    --out-every N             Write only every Nth resolved point. Thins the file, never
                              the computation: every step is still taken and kind=measure
                              still sees every point. [default: 1]
                              A stride is safe when it is far narrower than the narrowest
                              feature you intend to see, and lossy in a way nothing can
                              detect when it is not: an overshoot between two kept points
                              is simply absent from the file. Resolving a 46 ns transition
                              at a 40 fs step, --out-every 1000 still leaves 2500 points
                              across it and loses nothing real. Thinning a 20 ms run of a
                              switching converter by the same factor loses the switching.
                              When the feature is comparable to the step, or you do not
                              know, use --format samples: it keeps both extremes of every
                              interval, at about 3% of the run's size.
                              Cannot be combined with --format samples -- the stride would
                              remove the peaks before the pyramid could record them, and
                              the file would still look complete.

EVENTS
    --events <path>           Where the JSONL event stream goes.
                              [default: <netlist stem>.events.jsonl]
    --events-text <path>      Where its human-readable twin goes. Moves independently of
                              --events. [default: <netlist stem>.events.txt]
                              Both are written only for a netlist that declares rules.
    --fail-on <severity>      The lowest severity that makes this process exit 2:
                              info, warning, error or fatal. [default: error]
                              A rule that ends the run still exits 3, and a
                              non-finite state still exits 4, whatever this is set to.

CHECKPOINTS
    --checkpoint-out <file>   Write the run's complete state at --tfinal, so a later run
                              can --resume it and continue bit-identically.
    --checkpoint-every T      Also rewrite that file every T seconds of simulated time, so
                              a killed run loses at most T.
    --resume <file>           Continue from a checkpoint. --tfinal stays absolute, and a
                              checkpoint only loads into the deck it came from.

OTHER
    --devices <file>          Read PWL-device and block declarations from a second file
                              instead of the netlist itself.
    -h, --help                Print this help.
    -V, --version             Print the version.

EXAMPLES
    general-simulator rc.cir --mode transient --tfinal 20e-3 --dt 10e-9 > out.csv

    # A long switching run: compute every step, write a thousandth of them.
    general-simulator conv.cir --mode transient --tfinal 20e-3 --dt 10e-9 --out-every 1000

    # Same data as a binary rawfile instead of CSV.
    general-simulator conv.cir --mode transient --tfinal 1e-3 --dt 1e-9 --format raw --out run.raw

NOTES
    kind=measure lines write their results to <netlist stem>.log, whatever --format says.
    kind=pyblock and kind=pyfunc need a `--features python` build of this binary.
    Full documentation is in book/user-guide/ -- start with cli-reference.md and
    time-stepping.md.",
        version = version_line()
    )
}

fn usage() -> String {
    "usage: general-simulator <netlist> [--devices <file>] [--mode dc|transient] [--tfinal T] \
     [--dt DT | --dt-max T --dt-min T --dt-init T --reltol R --abstol A] [--max-steps N] \
     [--format csv|raw|samples] [--out <path>] [--out-every N]\n\
     [--events <path>] [--events-text <path>] [--fail-on <severity>]\n\
     [--checkpoint-out <file>] [--checkpoint-every T] [--resume <file>]\n\
     \n\
     --checkpoint-out writes the run's complete state at --tfinal (and, with --checkpoint-every T, \
     every T seconds of simulated time, replacing the same file) so a later run can --resume it \
     and continue to a larger --tfinal bit-identically to a run that never stopped. A checkpoint \
     only loads into the deck it came from.\n\
     \n\
     --dt fixes the step size every step (deterministic, exactly reproducible). Omit it (and \
     optionally tune --dt-max/--dt-min/--dt-init/--reltol/--abstol) for adaptive step-size \
     control instead — small steps where the solution is changing fast, large steps where it's \
     settled, the same local-truncation-error approach every real SPICE-family tool uses by \
     default. --dt and any --dt-*/--reltol/--abstol flag are mutually exclusive.\n\
     \n\
     --max-steps N caps how many accepted TimeStep::Adaptive steps one run may take before \
     aborting with an error, instead of running unboundedly (each accepted step holds its own \
     row in memory for the rest of the run -- an adaptive step-size controller that's stalled, \
     whether from a real bug or a pathological netlist/tolerance combination, would otherwise \
     grow that memory without limit until the process, or the whole machine, runs out of it -- \
     this happened on a real run before this flag existed). Defaults to 10,000,000, far above \
     any legitimate run this project has needed; only relevant under adaptive stepping (--dt \
     runs a fixed, precomputed number of steps and can't stall this way).\n\
     \n\
     --format selects the output serialization: 'csv' (the default, unchanged from before this \
     flag existed) prints t,V(node1),V(node2),...,<block names> to stdout, one row per resolved \
     point. 'raw' writes the same data as a binary SPICE rawfile instead (the format ngspice and \
     the wider SPICE-tooling ecosystem, including Python readers such as PySpice, read and \
     write) -- since a binary format has nowhere sensible to go on a terminal, this always \
     writes to a file: --out <path> if given, otherwise <netlist stem>.raw next to the input \
     file.\n\
     \n\
     --out-every N writes only every Nth resolved point (default 1, i.e. every point). It \
     thins the output, never the computation: every step is still taken, the solution is \
     unchanged, and kind=measure still sees every point -- only serialization is skipped. This \
     exists because the timestep and the useful output rate can legitimately differ by orders \
     of magnitude: verifying zero-voltage switching needs a step below r_on*C_oss (0.4 ps at \
     1 mOhm and 400 pF), which is 3.75 M steps across a 150 ns commutation window, while the \
     measurement itself needs a few thousand points. Without this flag that run is affordable \
     to compute and not to write down.\n\
     \n\
     --events <path> and --events-text <path> say where the two event files go, instead of \
     <netlist stem>.events.jsonl and <netlist stem>.events.txt beside the netlist. They move \
     independently: a tool reads the JSONL and a person reads the text, and a caller that \
     redirects one should not be handed the other somewhere it never looks. Both are created \
     only for a netlist that declares rules, and a path that cannot be created is an error -- a \
     diagnostics channel that vanishes without a word is the failure these flags exist to fix.\n\
     \n\
     Any kind=measure lines in the netlist write their results to <netlist stem>.log, regardless \
     of --format -- see book/user-guide/src/measurements.md."
        .to_string()
}
