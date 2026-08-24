//! Resolves every MOSFET's gate state, each transient step, from a graph of named,
//! independently reusable `continuous-blocks` blocks (`Const`, `Time`, `Pwc`, `Pwl`, `Sin`,
//! `Pulse`, `Exp`, `Sffm`, `Sum`, `Gain`, `Pid`, `StateSpace`, `TransferFunction`, `Vco`, `Pwm`,
//! `PhaseShiftPwm`, `Product`, `Saturation`, `Table`, `MathFn1`/`2`/`3`, `Hysteresis`,
//! `CoordinateTransform`, `Pmsm`, `CScript`, `Probe`, `Sig2Gate`, `Sig2Voltage`, `Sig2Current`)
//! wired together by the caller — the same discipline a real block-diagram tool uses: an error
//! signal is a `Sum` block's output, a filtered-derivative PID compensator is a
//! `TransferFunction` given its own `N(s)/D(s)` coefficients, and a frequency-modulated PWM
//! carrier is `Pid -> Gain -> PhaseShiftPwm`, not a single function that bakes a specific
//! topology together.
//!
//! **There is no separate "closed-loop mode," and no non-block-driven gate at all** — every
//! [`GateBinding`] is `Block(name)`, reading a named block's current output (`>= 0.5` means
//! on), whether that block's own input chain traces back to a
//! [`BlockKind::Probe`] of the circuit's own state (the historically "closed-loop" case) or is
//! just a `Const` (the historically "open-loop, fixed" case, now expressed as an ordinary block
//! instead of a special no-block `GateBinding` variant) — both are resolved by exactly the same
//! code, every step, because from the solver's point of view they're the same kind of question:
//! "what's this device's terminal condition right now," answered from whatever the netlist and
//! device file actually say. A real circuit simulator doesn't have a closed-loop *mode* either —
//! closed-loop is a property of how a circuit happens to be wired, not an analysis type the tool
//! needs to be told about upfront; `.op` and `.tran` are genuinely distinct analyses (different
//! equations solved), but nothing about *this* function is analysis-specific.
//!
//! Sampled-data co-simulation: every block reads the *previous* circuit step's measurement,
//! the whole graph is evaluated once per circuit step (in the causal order [`topological_order`]
//! derives from the graph's own `Signal::Block` dependencies — declaration order is not the
//! evaluation order; see that function and [`BlockInstance`]), and the result decides gate
//! states before the circuit step itself is solved. [`crate::simulate_closed_loop`] (a single
//! fixed `Sum`-then-`Pid`-then-duty-
//! comparator topology, named for the one case it was first built for) remains as a lighter
//! Rust-level convenience for simple direct callers; this module is the general one, used by
//! `elspice-pwl-cli` unconditionally for every MOSFET-containing transient run.

use std::collections::BTreeMap;

use continuous_blocks::{
    math_ops, CoordinateTransform, Hysteresis, MathFn1, MathFn2, MathFn3, Pid, Pmsm, StateSpace,
    TransferFunction, Vco,
};
use cscript_ffi::CScriptRegistry;
use pwl_devices::{Diode, Mosfet};
use spice_core::Dialect;

use crate::{
    classify_segments, sawtooth_carrier, step_control, step_with_fallback, DaeError, GateState,
    OperatingPoint, Segment, TimeStep, TransientFunction, RINGING_COOLDOWN_STEPS,
};

/// Where a block's input value comes from: another block's output this same step, or that (or
/// any) block's own output from the *previous* step. There is deliberately **no** variant that
/// reads a circuit quantity (`V(node)`/`I(branch)`) directly — that crossing from the physical
/// domain into the signal domain must go through an explicit, named [`BlockKind::Probe`] block
/// instead (referenced afterward like any other block, via `Signal::Block`). See
/// [`BlockKind::Probe`]'s own doc comment for why this boundary is enforced rather than
/// implicit, the same way a reference tool/Simscape requires an explicit PS-a reference tool Converter block
/// between a physical port and a signal port instead of wiring them together directly.
#[derive(Debug, Clone, PartialEq)]
pub enum Signal {
    /// Another block's output *this* step — may name any block in the same slice, declared
    /// before or after this one: [`topological_order`] derives each step's actual evaluation
    /// order from the full `Signal::Block` dependency graph, not declaration position, so
    /// "before/after" in the source text no longer has to match causal order (see that
    /// function). A genuine same-step cycle among these edges (`A` depends on `B` depends on
    /// `A`, however indirectly, including a block naming itself) is a model error reported as
    /// `DaeError::AlgebraicLoop` before any step is solved — see [`BlockInstance`]'s own doc
    /// comment.
    Block(String),
    /// A named block's own output from the *previous* step (`0.0` before the first step,
    /// matching every dynamic block's own "starts at rest" convention). Unlike `Signal::Block`,
    /// this is not a same-step dependency at all — it reads state fixed before this step even
    /// starts — so it never contributes an edge to the dependency graph [`topological_order`]
    /// builds, and is consequently the *sanctioned* way to close what would otherwise be a
    /// same-step cycle (e.g. a current controller regulating a [`BlockKind::Pmsm`]'s own
    /// `id`/`iq` outputs, or a PLL's angle estimate feeding the very
    /// [`BlockKind::CoordinateTransform`] `Park` block that produced its own error signal): the
    /// one-sample delay every real digital controller reading its own last output already has.
    BlockPrev(String),
}

/// What a [`BlockKind::Probe`] reads from the circuit's own previous-step operating point —
/// `V(node)` or `I(branch)`, anything [`OperatingPoint::value`] accepts, keyed by exactly the
/// same `V(...)`/`I(...)` naming convention `elspice-mna` itself uses for MNA unknowns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeTarget {
    Voltage(String),
    Current(String),
}

/// A [`BlockKind::Pid`]'s anti-windup bound: fixed at model-build time, or read fresh from the
/// graph every step. `Fixed` is every existing use of `Pid` before this variant existed —
/// unchanged behavior, one input (the error signal) as always. `Dynamic` is for a controller
/// whose *achievable* output range genuinely depends on other, still-evolving state (e.g. a
/// current-loop PID commanding a pole voltage that can't physically exceed roughly half the
/// DC bus voltage, itself still rising during a soft-start ramp) — a `Fixed` bound sized for
/// the final steady-state range is badly oversized early on, so the PID's own anti-windup never
/// engages even though the real plant is already saturated far below that fixed bound,
/// producing sustained, hard-to-diagnose windup-driven oscillation (worked example: an
/// `elspice-pwl-pfc-three-phase-vsc` experiment's own three-phase active-front-end current loop,
/// in the sibling `internal-archive` repo). `Dynamic` reads two *extra* inputs beyond
/// the error signal, in order `(clamp_lo, clamp_hi)`, evaluated fresh every step exactly like
/// any other block input — see [`BlockInstance`]'s own doc comment for the resulting input
/// count.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PidClamp {
    Fixed(f64, f64),
    Dynamic,
}

/// One block's behavior. `Const`/`Pwc`/`Pwl`/`Sin`/`Pulse`/`Exp`/`Sffm` are sources (zero
/// inputs); `Sum`/`Gain` are stateless (recomputed fresh from their inputs every step);
/// `Pid`/`StateSpace`/`TransferFunction`/`Vco` carry their own state forward across steps.
#[derive(Debug, Clone, PartialEq)]
pub enum BlockKind {
    /// A fixed value, ignoring time — e.g. a nominal frequency or a fixed setpoint.
    Const(f64),
    /// The current step's own simulated time (seconds), zero inputs — the standard
    /// block-diagram "clock" source, needed to build a genuine `sin(2*pi*f*t)`-style time
    /// varying signal out of `MathFn1`/`Gain` blocks (there's otherwise no way for a block to
    /// see `t` directly; `Pwc`/`Pwl`'s own use of it is internal to those blocks alone).
    Time,
    /// A piecewise-**constant** function of time: the value from the last point at or before
    /// `t` (the first point's value for `t` before it). Used for reference schedules, including
    /// step tests (two points is a step at the second point's time). `repeat`, if `true`, wraps
    /// `t` into `[points[0].0, points.last().0)` (period = last time − first time) once `t`
    /// passes the last point, instead of holding flat forever — a periodic step/square-like
    /// waveform. **Not the same interpolation as [`BlockKind::Pwl`]** (this block is named
    /// `pwc` at the CLI level specifically to avoid the ambiguity a shared `pwl` name would
    /// create with the real piecewise-*linear* SPICE-matching source below — see that variant's
    /// own doc comment).
    Pwc {
        points: Vec<(f64, f64)>,
        repeat: bool,
    },
    /// A piecewise-**linear** function of time — real SPICE `PWL(t1 v1 t2 v2 ...)` semantics,
    /// linearly interpolated between breakpoints, held at the first/last point's value before/
    /// after the breakpoint range (matching `elspice_mna::TransientFunction::Pwl`'s own
    /// electrical-domain behavior exactly, so the same breakpoint list means the same waveform
    /// whether it drives a `V`/`I` source directly or a signal-domain reference through this
    /// block). `repeat`, if `true`, wraps `t` into `[points[0].0, points.last().0)` once past
    /// the last point instead of holding flat — the electrical-domain `PWL` source has no such
    /// option (SPICE's own repeat semantics aren't implemented there), so a genuinely periodic
    /// piecewise-linear waveform (a triangle/sawtooth reference, a repeating ramp) is only
    /// available here in the signal domain.
    Pwl {
        points: Vec<(f64, f64)>,
        repeat: bool,
    },
    /// One of the electrical domain's four other time-varying source forms
    /// (`elspice_mna::TransientFunction::Sin`/`Pulse`/`Exp`/`Sffm` — never `::Pwl`, which this
    /// module models as its own [`BlockKind::Pwl`] above instead, specifically to add the
    /// `repeat` option `TransientFunction` doesn't have), reused directly rather than
    /// reimplemented, so a `V`/`I` source and a signal-domain reference built from the same
    /// parameters produce bit-for-bit the same waveform. Zero inputs; evaluated via
    /// [`TransientFunction::value_at`] at the current step's own `t`.
    Waveform(TransientFunction),
    /// Weighted sum of its inputs, one sign per input (`+1.0`/`-1.0` for an error junction).
    Sum(Vec<f64>),
    /// Scales its single input.
    Gain(f64),
    /// A compiled PID with two-sided conditional-integration anti-windup against `clamp` — see
    /// [`crate::simulate_closed_loop`]'s doc comment for why two-sided anti-windup matters; the
    /// mechanism here is identical, just attached to this block instead of baked into a whole
    /// controller function. `clamp` is this PID's own notion of "my output is saturated,"
    /// independent of whatever downstream `Gain`/`Vco` blocks do to it after — same as a real
    /// PID block's own configured output limits. See [`PidClamp`] for the fixed-vs-dynamic
    /// choice and what it changes about this block's own input count.
    Pid { pid: Pid, clamp: PidClamp },
    /// An arbitrary single-input single-output continuous-time block given directly as its own
    /// `(A, B, C, D)` matrices — a compensator/filter that doesn't already have a named
    /// convenience constructor, e.g. a low-pass filter placed ahead of a `Pid` to damp a
    /// resonant plant. No anti-windup (that's specifically a `Pid` output's own concern, not
    /// every dynamic block's); stepped forward unconditionally every timestep via
    /// `StateSpace::rk4_step`.
    StateSpace(StateSpace),
    /// A single-input single-output block given as a rational `N(s)/D(s)` (numerator/
    /// denominator coefficients, highest-degree first) rather than a `Pid`'s `Kp`/`Ki`/`Kd`
    /// convenience parameterization — e.g. a hand-derived PID-with-filtered-derivative
    /// compensator (`C(s) = Kp + Ki/s + Kd*N*s/(s+N)`, put over one denominator first: a pure
    /// derivative term alone is non-causal/unrealizable, so every real PID, textbook or
    /// otherwise, filters it — see [`Pid::to_transfer_function`]'s own doc comment for the
    /// derivation). Compiled once via [`TransferFunction::to_state_space`]; no anti-windup, for
    /// the same reason `StateSpace` above has none.
    TransferFunction(TransferFunction),
    /// A voltage-controlled oscillator (see [`Vco`]) — a bare, standalone oscillator producing
    /// a `[0, 1)` ramp, still useful on its own (a raw frequency-to-ramp conversion for
    /// something other than gate control). Not how a gate-driving PWM modulator gets its own
    /// switching frequency, though — see [`BlockKind::Pwm`]/[`BlockKind::PhaseShiftPwm`] below,
    /// PWM Modulator 1/2, neither of which reads this block at all.
    Vco(Vco),
    /// **PWM Modulator 1**: fixed-frequency, duty-driven, **active-high complementary** PWM.
    /// One input, `duty` (`[0,1]`, clamped, read fresh every step from anywhere in the graph —
    /// a `Pid`, a filtered `TransferFunction`, a plain `Const`...), fixed carrier frequency
    /// `freq_hz`. Two outputs, following the [`BlockKind::CScript`] `output_names` convention:
    /// `output_names[0]` (aliasing this block's own `.name`) is the main signal, `output_names[1]`
    /// its active-high complement — the fusion of what used to be two separate `GateBinding`
    /// variants (`Pwm`/`PwmComplement`) into one component, per explicit request. `red`/`fed`
    /// (seconds) are independent per-edge dead-time delays — see
    /// [`math_ops::complementary_pwm_with_deadtime`] for the exact rising-edge-only-delay
    /// semantics and why `red=fed=0.0` recovers the ideal, gap-free, overlap-free pair exactly.
    /// Stateless: a pure function of `(t, duty)` every step, no internal oscillator.
    Pwm {
        freq_hz: f64,
        red: f64,
        fed: f64,
        output_names: Vec<String>,
    },
    /// **PWM Modulator 2**: frequency+phase+duty-driven, **active-high complementary** PWM —
    /// the fusion of what used to be two separate `GateBinding` variants (`Vco`/`VcoPhase`)
    /// plus a block-driven `duty` neither had, again with the same dead-time/complementary-
    /// output treatment as [`BlockKind::Pwm`]. This is *not* a variant of [`BlockKind::Vco`] —
    /// it owns its own frequency-integration state directly (`osc` reuses [`Vco`]'s own
    /// clamp-and-integrate math purely as an implementation detail, the same formula, not a
    /// shared block reference), so two instances fed the *same* `freq` input stay bit-for-bit
    /// phase-synchronized (deterministic integration, same `dt`, same starting phase `0.0`),
    /// the way e.g. a dual-active-bridge's two legs need to be, without a separately-declared
    /// shared oscillator block in between. Three inputs, in order: `freq` (Hz, clamped
    /// internally to `[osc.f_min, osc.f_max]`), `phase` (`[0,1)`, a phase-shift command as a
    /// fraction of one carrier period — *not* this block's own internal integration state, a
    /// different thing), `duty` (`[0,1]`, clamped). `red`/`fed` are in seconds, exactly like
    /// [`BlockKind::Pwm`]'s own, but converted to a phase fraction using *this step's own*
    /// resolved frequency (not a fixed constant) — this matters for a variable-frequency
    /// converter, since the same absolute dead time eats a larger fraction of the period at
    /// higher switching frequency, a real effect on e.g. a resonant converter's own ZVS margin,
    /// not just bookkeeping.
    PhaseShiftPwm {
        osc: Vco,
        red: f64,
        fed: f64,
        output_names: Vec<String>,
    },
    /// Multiplies all its inputs together (see [`math_ops::product`]).
    Product,
    /// Clamps its single input to `[-limit, limit]` (see [`math_ops::saturation`]).
    Saturation(f64),
    /// Linear interpolation through a fixed `(x, y)` table (see
    /// [`continuous_blocks::waveform_arithmetic::table`]) — a `table(x, a, b, c, d, ...)`-
    /// style lookup.
    Table(Vec<(f64, f64)>),
    /// One of the single-argument real waveform-arithmetic functions (`cos`, `sin`, `exp`,
    /// `sqrt`, ... — see [`continuous_blocks::waveform_arithmetic`] for the full list and what
    /// was deliberately left out).
    MathFn1(MathFn1),
    /// One of the two-argument real waveform-arithmetic functions (`atan2`, `hypot`, `pow`,
    /// `min`, `max`, ...).
    MathFn2(MathFn2),
    /// One of the three-argument real waveform-arithmetic functions (`if`, `limit`).
    MathFn3(MathFn3),
    /// A Schmitt-trigger comparator (see [`Hysteresis`]) — bang-bang/hysteresis-band control,
    /// used when there's no fixed switching frequency to modulate a duty command onto (unlike
    /// `Pid` feeding a [`BlockKind::Pwm`]). Its output is `1.0`/`0.0`, read directly by a
    /// [`GateBinding::Block`] rather than compared against a carrier.
    Hysteresis(Hysteresis),
    /// A dynamically-loaded, user-supplied block (see [`cscript_ffi`]): `lib` is a precompiled
    /// shared library exporting `cscript_start`/`cscript_output`/(optionally)`cscript_free`/
    /// `cscript_clone`, filling `output_names.len()` outputs. Unlike every other `BlockKind`,
    /// this one can carry state no Rust type here knows anything about — see [`cscript_ffi`]'s
    /// own module doc comment for the full C-side contract and why [`TimeStep::Adaptive`]
    /// requires `cscript_clone` to be exported.
    ///
    /// `sample_time`, if given, makes this block run on its *own* fixed-period sample grid
    /// (like a discrete controller block with a configured `Ts` in any block-diagram tool),
    /// independent of the circuit's own resolved step size: `cscript_output` is only actually
    /// called once accumulated time since the last call reaches `sample_time`, and this block's
    /// output holds its last value (zero-order hold) on every step in between — the right model
    /// for something like a fixed-frequency digital controller, which genuinely does not run at
    /// the power stage's own (much finer, and possibly adaptive/irregular) step rate. `None`
    /// (the default) calls `cscript_output` every resolved circuit step instead, passing that
    /// step's own `dt` — the right choice for a block meant to behave continuously.
    CScript {
        lib: std::path::PathBuf,
        output_names: Vec<String>,
        sample_time: Option<f64>,
    },
    /// One of the six Clarke/Park coordinate transforms (see
    /// [`continuous_blocks::CoordinateTransform`]) — the standard `abc`/`alpha-beta-0`/`d-q-0`
    /// change of basis used to regulate a three-phase quantity (grid-tied PFC, motor drive) with
    /// a `Pid` on a DC-like `d`/`q` value instead of chasing a sine wave directly. Stateless and
    /// multi-output, following exactly the same `output_names` convention [`BlockKind::CScript`]
    /// established: `inputs` supplies `kind.input_count()` values in the order
    /// [`CoordinateTransform::call`] expects, `output_names.len()` must equal 3 (this family's
    /// output count — see [`CoordinateTransform::output_names`] for the conventional per-
    /// transform names, e.g. `["alpha", "beta", "zero"]` for `Clarke`), the block's own `.name`
    /// binds to the first (primary) output, and the remaining two are inserted under their own
    /// `output_names` entries so a downstream block can reference them directly via
    /// `Signal::Block(name)`.
    CoordinateTransform {
        kind: CoordinateTransform,
        output_names: Vec<String>,
    },
    /// A permanent-magnet synchronous motor (see [`continuous_blocks::Pmsm`]) — genuinely
    /// nonlinear (bilinear speed/current coupling), so like [`BlockKind::Vco`] it carries its
    /// own state and is integrated via its own `step()` (RK4) rather than compiled to a
    /// [`StateSpace`]. Three inputs, in order: `vd`, `vq` (rotor-frame stator voltage commands,
    /// V — typically a `ClarkeParkInv`'s output, or a `CoordinateTransform` intermediate wired
    /// through a `Pid`), and `t_load` (N*m, the mechanical load torque). Four outputs, same
    /// `output_names` convention as [`BlockKind::CoordinateTransform`]/[`BlockKind::CScript`]
    /// (`output_names.len()` must be 4, the block's own name aliases the first/primary output):
    /// `id`, `iq` (A), `omega_m` (mechanical speed, rad/s), and `theta_e` (electrical angle,
    /// already wrapped to `[0, 2*pi)` via [`continuous_blocks::Pmsm::theta_e_wrapped`] — ready
    /// to feed a [`BlockKind::CoordinateTransform`] `Park`/`ClarkePark` block directly). Starts
    /// at rest (`id = iq = omega_m = theta_e = 0`) — no initial-condition override, matching
    /// every other dynamic block in this graph.
    Pmsm {
        pmsm: Pmsm,
        output_names: Vec<String>,
    },
    /// The **PS-to-Signal** converter: the *only* way a circuit quantity (`V(node)`/
    /// `I(branch)`) enters the signal domain. Zero block-graph inputs (it reads the circuit's
    /// own previous-step operating point directly, the same `point_prev` lookup a bare
    /// `meas:`-style reference used to do before this was enforced) — its value is then an
    /// ordinary block output, read by any downstream block via `Signal::Block(this_block's_name)`
    /// exactly like any other source block (`Const`/`Pwl`/`Time`). Modeled directly on
    /// a reference tool/Simscape's own PS-a reference tool Converter: a physical port and a signal port are
    /// type-distinct there and cannot be wired together without one of these in between: this
    /// is the same rule, enforced the same way, at the netlist level instead of a GUI's wiring
    /// canvas (a future UI enforcing the same rule visually is the intended companion, not a
    /// replacement for this).
    Probe(ProbeTarget),
    /// The **Signal-to-PS** converter for a discrete physical actuation: the *only* legal
    /// target for a [`GateBinding::Block`]'s own named block — `dae-runtime` rejects a
    /// `GateBinding` naming anything else with `DaeError::GateTargetNotSig2Gate`. Purely an
    /// identity pass-through numerically (`value =
    /// input`); its entire purpose is marking, at the netlist level, exactly where a signal
    /// stops being "just a number a controller computed" and starts being "a command that
    /// actuates a physical switch" — the discrete-actuation counterpart to
    /// [`BlockKind::Sig2Voltage`]/[`BlockKind::Sig2Current`]'s continuous case below. One input.
    Sig2Gate,
    /// The **Signal-to-PS** converter for a continuous quantity, closing the write-direction
    /// gap [`BlockKind::Probe`] doesn't (a probe only ever reads): the *only* legal way a
    /// signal-domain block's output drives an independent voltage source's own magnitude. A `V`
    /// element's own literal value field in the netlist names this block directly (e.g. `V1 a 0
    /// VDRV`, where `VDRV` is a declared `Sig2Voltage` block) — `elspice-mna`'s own
    /// `Expression::parse_scalar` already accepts a bare symbol there with no change needed on
    /// that side; `dae-runtime` requires, at validation time, that any such symbol naming a
    /// declared block resolve to exactly this kind (see
    /// `DaeError::SourceNotSig2PhysicalConverter`), and every step, substitutes this block's own
    /// just-computed output value into the circuit solve in that symbol's place — a real,
    /// bidirectional physical/control coupling `Signal::Measure`'s read-only predecessor could
    /// never express (see `elspice-pwl-buck-dc-motor-cascade`'s own README for the concrete gap
    /// this closes: a block could observe a circuit's voltage but never load it). Purely an
    /// identity pass-through numerically, same as [`BlockKind::Sig2Gate`]; the type-distinct
    /// name is what the enforcement (and, later, a UI) keys on. One input.
    Sig2Voltage,
    /// The [`BlockKind::Sig2Voltage`] counterpart for an `I` (independent current source)
    /// element's own literal value field. One input.
    Sig2Current,
}

/// One named block instance and where its inputs (if any) come from.
/// `Const`/`Pwc`/`Pwl`/`Waveform`/`Probe` blocks must have zero inputs; `Sum`/`Product` need one
/// input per sign/factor; `Gain`/
/// `StateSpace`/`TransferFunction`/`Vco`/`Saturation`/`Table`/`MathFn1`/`Sig2Gate`/
/// `Sig2Voltage`/`Sig2Current` each need exactly one; `Pid` needs exactly one (the error signal)
/// when its `clamp` is `PidClamp::Fixed`, or exactly three (`error, clamp_lo, clamp_hi`, in that
/// order) when `PidClamp::Dynamic`; `MathFn2` needs two; `MathFn3` needs three;
/// `CoordinateTransform` needs `kind.input_count()` (3 for `Clarke`/`ClarkeInv`, 4 for the
/// others — see [`continuous_blocks::CoordinateTransform::input_count`]); `Pmsm` needs exactly
/// three (`vd`, `vq`, `t_load`, in that order). Evaluated once per step in the causal order
/// [`topological_order`] derives from the slice's own `Signal::Block` dependency graph — *not*
/// the order the slice happens to be given in; a `Signal::Block` input may name any block in
/// the same slice regardless of declared position (source blocks, naturally, need none, and a
/// genuine cycle among these edges is rejected as `DaeError::AlgebraicLoop` before any step
/// runs — see `topological_order`'s own doc comment for how).
#[derive(Debug, Clone, PartialEq)]
pub struct BlockInstance {
    pub name: String,
    pub kind: BlockKind,
    pub inputs: Vec<Signal>,
}

/// How one MOSFET's gate state is resolved, every step: always from a named block's current
/// output, on while it's `>= 0.5`. No non-block-driven variant exists — even a permanently-off
/// gate is an explicit `Const(0.0)` wired through a [`BlockKind::Sig2Gate`], the same as every
/// other gate — and no bare carrier-comparator variant exists either: that comparison now lives
/// entirely inside gate-driving `BlockKind`s themselves ([`BlockKind::Pwm`]/
/// [`BlockKind::PhaseShiftPwm`], or a hand-built chain of ordinary blocks), so `GateBinding` has
/// exactly one job — reading a number and thresholding it — regardless of what produced that
/// number: a modulator's own main/complement output, a [`BlockKind::Hysteresis`] block (no
/// carrier at all, event-driven bang-bang switching), or any other block a caller composes.
#[derive(Debug, Clone, PartialEq)]
pub enum GateBinding {
    /// On while the named block's current output is `>= 0.5`.
    Block(String),
}

impl GateBinding {
    /// The block this binding reads from.
    fn source_blocks(&self) -> [Option<&str>; 2] {
        match self {
            GateBinding::Block(name) => [Some(name), None],
        }
    }

    fn resolve(&self, outputs: &BTreeMap<String, f64>) -> GateState {
        let GateBinding::Block(name) = self;
        if outputs[name.as_str()] >= 0.5 {
            GateState::On
        } else {
            GateState::Off
        }
    }
}

#[derive(Clone)]
enum BlockState {
    Stateless,
    /// Shared by `Pid`, `StateSpace`, and `TransferFunction` — all three are, underneath,
    /// "step this compiled `StateSpace` forward" with only `Pid` additionally applying
    /// anti-windup (see the match arm in the step loop below).
    Dynamic {
        state_space: continuous_blocks::StateSpace,
        x: Vec<f64>,
    },
    Vco {
        vco: Vco,
        phase: f64,
    },
    /// PWM Modulator 2's own frequency-integration state — structurally identical to `Vco`
    /// above (both reuse the same [`Vco`] clamp-and-integrate math), but a distinct variant so
    /// nothing in this module's own dispatch code has to pretend PWM Modulator 2 *is* a `Vco`
    /// block, which it isn't.
    PhaseShiftPwm {
        osc: Vco,
        phase: f64,
    },
    Pmsm {
        pmsm: Pmsm,
        x: [f64; 4],
    },
    Hysteresis {
        hysteresis: Hysteresis,
        on: bool,
    },
    /// A live, per-instance [`cscript_ffi::CScriptInstance`], plus the zero-order-hold
    /// bookkeeping [`BlockKind::CScript`]'s `sample_time` needs: `time_since_sample` accumulates
    /// circuit `dt`s until it reaches the configured sample period (or is always "due" every
    /// step when `sample_time` is `None`), and `last_output` is what gets returned/held on
    /// every step the block *doesn't* actually run `cscript_output`. Cloning this variant (only
    /// ever needed by [`TimeStep::Adaptive`]'s retry loop, which clones the whole
    /// `block_states` vector before every trial) calls into `cscript_clone` for `instance` and
    /// **panics** if the library doesn't export it — see
    /// [`simulate_transient_with_blocks`]'s own upfront check, which exists specifically so
    /// that panic is unreachable in practice.
    CScript {
        instance: cscript_ffi::CScriptInstance,
        time_since_sample: f64,
        last_output: Vec<f64>,
    },
}

/// One step's result: `(t, OperatingPoint, block_outputs)`, where `block_outputs` is every
/// block's value that step (by name) — useful for plotting a controller's internal signals
/// (e.g. a `Vco`'s commanded frequency) without needing to separately re-derive them.
pub type TransientWithBlocksStep = (f64, OperatingPoint, BTreeMap<String, f64>);

/// Name -> block index, including a [`BlockKind::CScript`]/[`BlockKind::CoordinateTransform`]/
/// [`BlockKind::Pmsm`]'s extra `output_names` (aliasing the index of the block that declared
/// them) — the one piece of bookkeeping both [`topological_order`] and the upfront gate-name
/// validation in [`simulate_transient_with_blocks`] need identically. Rejects any name — a
/// block's own `.name`, or one of its extra `output_names` aliases — that collides with one
/// already seen, rather than letting the later one silently win (see
/// [`DaeError::DuplicateBlockName`]'s own doc comment for why that matters).
fn block_index_by_name(blocks: &[BlockInstance]) -> Result<BTreeMap<&str, usize>, DaeError> {
    let mut index_of = BTreeMap::new();
    for (i, b) in blocks.iter().enumerate() {
        if index_of.insert(b.name.as_str(), i).is_some() {
            return Err(DaeError::DuplicateBlockName(b.name.clone()));
        }
        let extra: &[String] = match &b.kind {
            BlockKind::CScript { output_names, .. }
            | BlockKind::CoordinateTransform { output_names, .. }
            | BlockKind::Pmsm { output_names, .. }
            | BlockKind::Pwm { output_names, .. }
            | BlockKind::PhaseShiftPwm { output_names, .. } => output_names,
            _ => &[],
        };
        for name in extra.iter().skip(1) {
            if index_of.insert(name.as_str(), i).is_some() {
                return Err(DaeError::DuplicateBlockName(name.clone()));
            }
        }
    }
    Ok(index_of)
}

/// A short, human-readable name for a `BlockKind`, for error messages that need to say what a
/// mistargeted block actually is (e.g. `DaeError::GateTargetNotSig2Gate`/
/// `SourceNotSig2PhysicalConverter`) without dumping its full parameter set.
fn block_kind_name(kind: &BlockKind) -> &'static str {
    match kind {
        BlockKind::Const(_) => "const",
        BlockKind::Time => "time",
        BlockKind::Pwc { .. } => "pwc",
        BlockKind::Pwl { .. } => "pwl",
        BlockKind::Waveform(TransientFunction::Sin { .. }) => "sinwave",
        BlockKind::Waveform(TransientFunction::Pulse { .. }) => "pulsewave",
        BlockKind::Waveform(TransientFunction::Exp { .. }) => "expwave",
        BlockKind::Waveform(TransientFunction::Sffm { .. }) => "sffmwave",
        // Never actually constructed (elspice-pwl-cli only builds `Waveform` from
        // Sin/Pulse/Exp/Sffm — a Pwl-shaped waveform always goes through `BlockKind::Pwl`
        // above instead, since only that variant supports `repeat`), but `TransientFunction`
        // is a 5-variant enum so this match must still be exhaustive.
        BlockKind::Waveform(TransientFunction::Pwl(_)) => "waveform(pwl)",
        BlockKind::Sum(_) => "sum",
        BlockKind::Gain(_) => "gain",
        BlockKind::Pid { .. } => "pid",
        BlockKind::StateSpace(_) => "statespace",
        BlockKind::TransferFunction(_) => "tf",
        BlockKind::Vco(_) => "vco",
        BlockKind::Pwm { .. } => "pwm",
        BlockKind::PhaseShiftPwm { .. } => "pspwm",
        BlockKind::Product => "product",
        BlockKind::Saturation(_) => "saturation",
        BlockKind::Table(_) => "table",
        BlockKind::MathFn1(f) => f.name(),
        BlockKind::MathFn2(f) => f.name(),
        BlockKind::MathFn3(f) => f.name(),
        BlockKind::Hysteresis(_) => "hysteresis",
        BlockKind::CScript { .. } => "cscript",
        BlockKind::CoordinateTransform { kind, .. } => kind.name(),
        BlockKind::Pmsm { .. } => "pmsm",
        BlockKind::Probe(_) => "probe",
        BlockKind::Sig2Gate => "sig2gate",
        BlockKind::Sig2Voltage => "sig2voltage",
        BlockKind::Sig2Current => "sig2current",
    }
}

/// Derives the causal evaluation order for `blocks` from their own `Signal::Block` dependencies
/// — a topological sort of the same-step dependency graph — instead of relying on declaration
/// order: a block may name another declared anywhere in the same slice, before or after it.
/// `BlockKind::Probe`/`Signal::BlockPrev` inputs never contribute a dependency edge here —
/// `Probe` has zero inputs (a source block, like `Const`/`Time`) and `BlockPrev` reads state
/// from strictly before this step (any block's own previous output), so neither can ever
/// participate in a same-step cycle by construction — exactly the reason `BlockPrev` exists. A
/// name that doesn't resolve to any block is not
/// this function's concern; it's reported at evaluation time instead (`DaeError::
/// UnknownBlockInput`, from `evaluate_blocks`' own `resolve` closure), so a typo and a
/// deliberately external reference are diagnosed the same way regardless of graph shape.
///
/// Implementation: classic DFS-based topological sort with three-color marking (white/gray/
/// black), chosen over Kahn's algorithm specifically because a discovered cycle can be reported
/// as the *exact path* that closes it (`["A", "B", "C", "A"]`, read as "A depends on B depends
/// on C depends on A") rather than just the set of nodes Kahn's leaves stranded with nonzero
/// in-degree when it gets stuck.
fn topological_order(blocks: &[BlockInstance]) -> Result<Vec<usize>, DaeError> {
    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        White,
        Gray,
        Black,
    }

    fn visit(
        i: usize,
        blocks: &[BlockInstance],
        index_of: &BTreeMap<&str, usize>,
        color: &mut [Color],
        stack: &mut Vec<usize>,
        order: &mut Vec<usize>,
    ) -> Result<(), DaeError> {
        match color[i] {
            Color::Black => return Ok(()),
            Color::Gray => {
                // A back-edge into a node already on the current DFS path: the cycle is that
                // node onward through the rest of the path, with it repeated at the end to show
                // the loop closing.
                let start = stack.iter().position(|&s| s == i).expect(
                    "a Gray node is always still on the stack by this function's own invariant",
                );
                let mut cycle: Vec<String> = stack[start..]
                    .iter()
                    .map(|&idx| blocks[idx].name.clone())
                    .collect();
                cycle.push(blocks[i].name.clone());
                return Err(DaeError::AlgebraicLoop { cycle });
            }
            Color::White => {}
        }
        color[i] = Color::Gray;
        stack.push(i);
        for signal in &blocks[i].inputs {
            if let Signal::Block(name) = signal {
                if let Some(&dep) = index_of.get(name.as_str()) {
                    visit(dep, blocks, index_of, color, stack, order)?;
                }
            }
        }
        stack.pop();
        color[i] = Color::Black;
        order.push(i);
        Ok(())
    }

    let index_of = block_index_by_name(blocks)?;
    let mut color = vec![Color::White; blocks.len()];
    let mut order = Vec::with_capacity(blocks.len());
    let mut stack = Vec::new();
    for i in 0..blocks.len() {
        if color[i] == Color::White {
            visit(i, blocks, &index_of, &mut color, &mut stack, &mut order)?;
        }
    }
    Ok(order)
}

/// Maps simulated time `t` into the effective evaluation time for a [`BlockKind::Pwc`]/
/// [`BlockKind::Pwl`] block: unchanged when `repeat` is `false`, or `points` has fewer than two
/// breakpoints (no well-defined period), or `t` hasn't yet reached the last breakpoint;
/// otherwise wraps `t` into `[points[0].0, points.last().0)`, period = last breakpoint time −
/// first breakpoint time, so the same finite breakpoint list repeats forever instead of holding
/// its last value flat — the periodic PWL/PWC source this session added specifically because
/// neither `elspice-mna`'s own electrical-domain `PWL(...)` source nor any prior signal-domain
/// block had a repeat option.
fn periodic_time(t: f64, points: &[(f64, f64)], repeat: bool) -> f64 {
    if !repeat || points.len() < 2 {
        return t;
    }
    let t0 = points[0].0;
    let t_last = points[points.len() - 1].0;
    let period = t_last - t0;
    if period <= 0.0 || t < t_last {
        return t;
    }
    t0 + (t - t0) % period
}

/// Evaluates every block once, in the causal `order` [`topological_order`] derived from the
/// graph's own `Signal::Block` dependencies (not declaration order), advancing `block_states`
/// in place at step size `dt`, reading any [`BlockKind::Probe`] from `point_prev` and any
/// [`Signal::BlockPrev`] from `prev_outputs` (the previous call's returned map; an empty map on
/// the very first step, so every `BlockPrev` reference is `0.0` there) — the one piece of
/// per-step work both [`TimeStep::Fixed`] and [`TimeStep::Adaptive`] need identically, factored
/// out so the adaptive loop below can re-run it (against a *cloned* `block_states`) once per
/// retry at a shrinking trial `dt`, without duplicating the block-dispatch match arms.
/// Shared by [`BlockKind::Pwm`] and [`BlockKind::PhaseShiftPwm`]'s own `evaluate_blocks` arms:
/// both modulators produce an **active-high complementary pair**, binding the primary output
/// under the block's own name (the caller's job, this just returns it) and the secondary
/// `complement` output under `output_names[1]` (inserted here, if given). One implementation so
/// a third gate-driving modulator only has to call this, not re-derive the convention.
fn emit_complementary_pair(
    outputs: &mut BTreeMap<String, f64>,
    output_names: &[String],
    main: bool,
    complement: bool,
) -> f64 {
    if let Some(name) = output_names.get(1) {
        outputs.insert(name.clone(), if complement { 1.0 } else { 0.0 });
    }
    if main {
        1.0
    } else {
        0.0
    }
}

fn evaluate_blocks(
    blocks: &[BlockInstance],
    order: &[usize],
    block_states: &mut [BlockState],
    point_prev: &OperatingPoint,
    prev_outputs: &BTreeMap<String, f64>,
    t: f64,
    dt: f64,
) -> Result<BTreeMap<String, f64>, DaeError> {
    let mut outputs: BTreeMap<String, f64> = BTreeMap::new();
    let resolve = |outputs: &BTreeMap<String, f64>, signal: &Signal| -> Result<f64, DaeError> {
        match signal {
            Signal::Block(name) => outputs
                .get(name)
                .copied()
                .ok_or_else(|| DaeError::UnknownBlockInput(name.clone())),
            Signal::BlockPrev(name) => Ok(prev_outputs.get(name).copied().unwrap_or(0.0)),
        }
    };

    for &i in order {
        let block = &blocks[i];
        let state = &mut block_states[i];
        let input_vals: Vec<f64> = block
            .inputs
            .iter()
            .map(|s| resolve(&outputs, s))
            .collect::<Result<_, _>>()?;

        let value = match (&block.kind, state) {
            (BlockKind::Const(v), _) => *v,
            (BlockKind::Time, _) => t,
            (BlockKind::Probe(target), _) => match target {
                ProbeTarget::Voltage(node) => {
                    point_prev.value(&format!("V({node})")).unwrap_or(0.0)
                }
                ProbeTarget::Current(branch) => {
                    point_prev.value(&format!("I({branch})")).unwrap_or(0.0)
                }
            },
            (BlockKind::Sig2Gate | BlockKind::Sig2Voltage | BlockKind::Sig2Current, _) => {
                input_vals[0]
            }
            (BlockKind::Pwc { points, repeat }, _) => {
                let t_eval = periodic_time(t, points, *repeat);
                let mut v = points.first().map(|(_, v)| *v).unwrap_or(0.0);
                for &(t_i, v_i) in points {
                    if t_i <= t_eval {
                        v = v_i;
                    } else {
                        break;
                    }
                }
                v
            }
            (BlockKind::Pwl { points, repeat }, _) => {
                let t_eval = periodic_time(t, points, *repeat);
                match points.as_slice() {
                    [] => 0.0,
                    [(_, v)] => *v,
                    _ if t_eval <= points[0].0 => points[0].1,
                    _ if t_eval >= points[points.len() - 1].0 => points[points.len() - 1].1,
                    _ => {
                        let mut v = points[points.len() - 1].1;
                        for window in points.windows(2) {
                            let (t0, v0) = window[0];
                            let (t1, v1) = window[1];
                            if t_eval >= t0 && t_eval <= t1 {
                                v = if (t1 - t0).abs() < f64::EPSILON {
                                    v1
                                } else {
                                    v0 + (v1 - v0) * ((t_eval - t0) / (t1 - t0))
                                };
                                break;
                            }
                        }
                        v
                    }
                }
            }
            (BlockKind::Waveform(f), _) => f.value_at(t),
            (BlockKind::Sum(signs), _) => math_ops::sum(&input_vals, signs),
            (BlockKind::Gain(k), _) => math_ops::gain(*k, input_vals[0]),
            (BlockKind::Product, _) => math_ops::product(&input_vals),
            (
                BlockKind::Pwm {
                    freq_hz,
                    red,
                    fed,
                    output_names,
                },
                _,
            ) => {
                let theta = sawtooth_carrier(t, *freq_hz);
                let duty = input_vals[0];
                let (main, complement) = math_ops::complementary_pwm_with_deadtime(
                    theta,
                    duty,
                    red * freq_hz,
                    fed * freq_hz,
                );
                emit_complementary_pair(&mut outputs, output_names, main, complement)
            }
            (BlockKind::Saturation(limit), _) => math_ops::saturation(input_vals[0], *limit),
            (BlockKind::Table(points), _) => {
                continuous_blocks::waveform_arithmetic::table(input_vals[0], points)
            }
            (BlockKind::MathFn1(f), _) => f.call(input_vals[0]),
            (BlockKind::MathFn2(f), _) => f.call(input_vals[0], input_vals[1]),
            (BlockKind::MathFn3(f), _) => f.call(input_vals[0], input_vals[1], input_vals[2]),
            (BlockKind::CoordinateTransform { kind, output_names }, _) => {
                let outs = kind.call(&input_vals);
                // Same convention as BlockKind::CScript below: the block's own name is bound to
                // the primary (first) output, any remaining output_names are inserted directly
                // under their own names.
                for (name, v) in output_names.iter().zip(outs.iter()).skip(1) {
                    outputs.insert(name.clone(), *v);
                }
                outs[0]
            }
            (BlockKind::Pid { clamp, .. }, BlockState::Dynamic { state_space, x }) => {
                let (lo, hi) = match clamp {
                    PidClamp::Fixed(lo, hi) => (*lo, *hi),
                    PidClamp::Dynamic => (input_vals[1], input_vals[2]),
                };
                let error = [input_vals[0]];
                let tentative_x = state_space.rk4_step(x, &error, dt);
                let tentative_output = state_space.output(&tentative_x, &error)[0];
                let saturating_further = (tentative_output >= hi && error[0] > 0.0)
                    || (tentative_output <= lo && error[0] < 0.0);
                if saturating_further {
                    state_space.output(x, &error)[0].clamp(lo, hi)
                } else {
                    *x = tentative_x;
                    tentative_output
                }
            }
            (
                BlockKind::StateSpace(_) | BlockKind::TransferFunction(_),
                BlockState::Dynamic { state_space, x },
            ) => {
                let u = [input_vals[0]];
                *x = state_space.rk4_step(x, &u, dt);
                state_space.output(x, &u)[0]
            }
            (BlockKind::Vco(_), BlockState::Vco { vco, phase }) => {
                *phase = vco.step(*phase, input_vals[0], dt);
                *phase
            }
            (
                BlockKind::PhaseShiftPwm {
                    red,
                    fed,
                    output_names,
                    ..
                },
                BlockState::PhaseShiftPwm { osc, phase },
            ) => {
                let freq_command = input_vals[0];
                let phase_offset = input_vals[1];
                let duty = input_vals[2];
                *phase = osc.step(*phase, freq_command, dt);
                let actual_freq = freq_command.clamp(osc.f_min, osc.f_max);
                let theta = (*phase + phase_offset).rem_euclid(1.0);
                let (main, complement) = math_ops::complementary_pwm_with_deadtime(
                    theta,
                    duty,
                    red * actual_freq,
                    fed * actual_freq,
                );
                emit_complementary_pair(&mut outputs, output_names, main, complement)
            }
            (BlockKind::Pmsm { output_names, .. }, BlockState::Pmsm { pmsm, x }) => {
                *x = pmsm.step(*x, input_vals[0], input_vals[1], input_vals[2], dt);
                let [id, iq, omega_m, theta_e] = *x;
                let full = [id, iq, omega_m, Pmsm::theta_e_wrapped(theta_e)];
                for (name, v) in output_names.iter().zip(full.iter()).skip(1) {
                    outputs.insert(name.clone(), *v);
                }
                full[0]
            }
            (BlockKind::Hysteresis(_), BlockState::Hysteresis { hysteresis, on }) => {
                *on = hysteresis.step(*on, input_vals[0]);
                if *on {
                    1.0
                } else {
                    0.0
                }
            }
            (
                BlockKind::CScript {
                    output_names,
                    sample_time,
                    ..
                },
                BlockState::CScript {
                    instance,
                    time_since_sample,
                    last_output,
                },
            ) => {
                // sample_time = None: run every step, exactly like every other dynamic block.
                // sample_time = Some(ts): accumulate circuit dt until ts is reached, then run
                // once with the *accumulated* elapsed time as this call's dt (not the much
                // finer circuit dt), and hold the result (zero-order hold) on every step in
                // between -- see BlockKind::CScript's own doc comment for why.
                let (due, elapsed) = match sample_time {
                    None => (true, dt),
                    Some(ts) => {
                        *time_since_sample += dt;
                        if *time_since_sample + 1e-15 >= *ts {
                            let elapsed = *time_since_sample;
                            *time_since_sample = 0.0;
                            (true, elapsed)
                        } else {
                            (false, 0.0)
                        }
                    }
                };
                if due {
                    *last_output = instance.call(&input_vals, elapsed, output_names.len());
                }
                // The primary value (this block's own name) is last_output[0], inserted below
                // like every other block; any additional declared output_names are inserted
                // here under their own names, so a downstream block can reference them
                // directly via Signal::Block(name) without needing to know they came from a
                // CScript block.
                for (name, v) in output_names.iter().zip(last_output.iter()).skip(1) {
                    outputs.insert(name.clone(), *v);
                }
                last_output.first().copied().unwrap_or(0.0)
            }
            _ => unreachable!("BlockState variant always matches its BlockKind"),
        };
        outputs.insert(block.name.clone(), value);
    }
    Ok(outputs)
}

fn resolve_gates(
    gates: &BTreeMap<String, GateBinding>,
    outputs: &BTreeMap<String, f64>,
) -> BTreeMap<String, GateState> {
    gates
        .iter()
        .map(|(mosfet_name, binding)| (mosfet_name.clone(), binding.resolve(outputs)))
        .collect()
}

/// Runs a transient with every MOSFET's gate resolved from a [`GateBinding`] each step — see
/// this module's doc comment for why there's no separate "closed-loop" entry point: a device
/// gated by a plain [`BlockKind::Hysteresis`] and one gated by a `Sum`-`Pid`-[`BlockKind::PhaseShiftPwm`]
/// chain that happens to read a [`BlockKind::Probe`] are resolved by exactly the same loop
/// below — every [`GateBinding`] is `Block(name)`, reading whatever `name`'s current output
/// happens to be. `step` picks fixed or adaptive timing — see [`TimeStep`]/[`AdaptiveConfig`]. Adaptive mode here can't
/// reuse [`step_control::adaptive_step`] directly (that assumes a `dt`-independent `system`):
/// this circuit's gate states, and so its `system`, are themselves a function of `dt` through
/// the block graph, so a rejected trial has to redo block evaluation *and* gate resolution at
/// the smaller `dt`, not just re-solve the same system — see the retry loop below.
#[allow(clippy::too_many_arguments)]
pub fn simulate_transient_with_blocks(
    source: &str,
    dialect: Dialect,
    diodes: &BTreeMap<String, Diode>,
    mosfets: &BTreeMap<String, Mosfet>,
    blocks: &[BlockInstance],
    gates: &BTreeMap<String, GateBinding>,
    shared_r_on: f64,
    x_initial: Option<&[f64]>,
    t_final: f64,
    step: TimeStep,
) -> Result<Vec<TransientWithBlocksStep>, DaeError> {
    // A CScript, CoordinateTransform, or Pmsm block's own `.name` is only its *primary* output
    // alias; `output_names` may also register extra named outputs (see evaluate_blocks' arms
    // for all three) that a gate binding is just as free to reference directly -- all need to
    // count as "known" here, or a valid netlist referencing one of those extra names gets
    // rejected before it ever runs.
    let block_names = block_index_by_name(blocks)?;
    for (mosfet_name, binding) in gates {
        for needed in binding.source_blocks().into_iter().flatten() {
            let Some(&idx) = block_names.get(needed) else {
                return Err(DaeError::UnknownBlockInput(needed.to_string()));
            };
            // A GateBinding's target must be an explicit Sig2Gate converter, never a raw
            // control block directly -- the enforced Signal-to-PS boundary for a discrete
            // physical actuation (see BlockKind::Sig2Gate's own doc comment). This also
            // correctly rejects naming a CScript/CoordinateTransform/Pmsm block's own *extra*
            // output alias directly (block_names maps those to the same index, whose kind is
            // never Sig2Gate), so no separate check is needed for that case.
            if !matches!(blocks[idx].kind, BlockKind::Sig2Gate) {
                return Err(DaeError::GateTargetNotSig2Gate {
                    gate: mosfet_name.clone(),
                    block: needed.to_string(),
                    found_kind: block_kind_name(&blocks[idx].kind),
                });
            }
        }
    }

    // Derive this step's (every step's -- the graph's shape never changes mid-run) causal
    // evaluation order up front, before any block state or circuit system is built: a cycle is
    // a model error the caller should hear about immediately, not partway through a possibly
    // long transient.
    let order = topological_order(blocks)?;

    // Only used to learn the system's `unknowns` ordering/count for the pre-first-step
    // `point_prev` below and the default `x_initial` — no step is solved with it. Solving one
    // would perturb `x_prev` away from `x_initial` before the real first step even runs, a
    // real behavioral difference from `simulate_transient_with_mosfets` for the plain
    // fixed/PWM case (caught by exactly that mismatch: this function must reduce to identical
    // numbers as that one whenever no block reads a `BlockKind::Probe`).
    let initial_states: BTreeMap<String, (Mosfet, GateState)> = mosfets
        .iter()
        .map(|(name, m)| (name.clone(), (*m, GateState::Off)))
        .collect();
    let (system0, _) =
        crate::build_with_mosfets(source, dialect, diodes, &initial_states, shared_r_on)?;

    // Signal-to-PS enforcement for a `V`/`I` source's own literal value: `elspice-mna` already
    // accepts a bare symbol there (`Expression::Symbol`), stamped verbatim into that source's
    // own `input_values` entry -- a genuinely time-varying `TransientFunction` source uses this
    // too, with the symbol set to the source's *own* element name (`sym == name`, resolved via
    // `system.transient_sources`, an entirely separate mechanism this check must not confuse
    // with a block-driven source). Any *other* symbol is a block-driven source candidate: it
    // must name a declared block, and that block must be the matching `Sig2Voltage`/
    // `Sig2Current` converter (never a raw control block directly) -- source element names are
    // always their own SPICE device letter (`V`/`I`) by construction, which is what picks the
    // expected converter kind below. V/I source stamps never depend on switch state, so
    // checking `system0` once here is representative of every later per-step rebuild.
    for (name, expr) in system0.inputs.iter().zip(&system0.input_values) {
        let elspice_mna::Expression::Symbol(sym) = expr else {
            continue;
        };
        if sym == name || system0.transient_sources.contains_key(name) {
            continue;
        }
        let Some(&idx) = block_names.get(sym.as_str()) else {
            continue;
        };
        let expected = match name.chars().next() {
            Some('V') | Some('v') => (BlockKind::Sig2Voltage, "sig2voltage"),
            Some('I') | Some('i') => (BlockKind::Sig2Current, "sig2current"),
            _ => continue,
        };
        if blocks[idx].kind != expected.0 {
            return Err(DaeError::SourceNotSig2PhysicalConverter {
                source: name.clone(),
                block: sym.clone(),
                expected_kind: expected.1,
                found_kind: block_kind_name(&blocks[idx].kind),
            });
        }
    }

    let mut x_prev = match x_initial {
        Some(x) => x.to_vec(),
        None => vec![0.0; system0.order()],
    };
    let mut point_prev = OperatingPoint {
        unknowns: system0.unknowns.clone(),
        x: x_prev.clone(),
        diode_names: Vec::new(),
        diode_z: Vec::new(),
        diode_raw_ioff: Vec::new(),
    };
    let mut x_prev_prev: Option<Vec<f64>> = None;

    let mut cscript_registry = CScriptRegistry::new();
    let mut block_states: Vec<BlockState> = Vec::with_capacity(blocks.len());
    for b in blocks {
        let dynamic = |state_space: StateSpace| {
            let x = vec![0.0; state_space.states()];
            BlockState::Dynamic { state_space, x }
        };
        let state = match &b.kind {
            BlockKind::Pid { pid, .. } => dynamic(pid.to_state_space()),
            BlockKind::StateSpace(ss) => dynamic(ss.clone()),
            BlockKind::TransferFunction(tf) => dynamic(tf.to_state_space()),
            BlockKind::Vco(vco) => BlockState::Vco {
                vco: *vco,
                phase: 0.0,
            },
            BlockKind::PhaseShiftPwm { osc, .. } => BlockState::PhaseShiftPwm {
                osc: *osc,
                phase: 0.0,
            },
            BlockKind::Pmsm { pmsm, .. } => BlockState::Pmsm {
                pmsm: *pmsm,
                x: [0.0; 4],
            },
            BlockKind::Hysteresis(hysteresis) => BlockState::Hysteresis {
                hysteresis: *hysteresis,
                on: false,
            },
            BlockKind::CScript {
                lib,
                output_names,
                sample_time,
            } => {
                let instance = cscript_registry
                    .instantiate(lib)
                    .map_err(DaeError::CScript)?;
                if matches!(step, TimeStep::Adaptive(_)) && !instance.supports_clone() {
                    return Err(DaeError::CScriptRequiresCloneForAdaptiveStep {
                        block_name: b.name.clone(),
                    });
                }
                BlockState::CScript {
                    instance,
                    // Initialized to `sample_time` itself (not 0.0, and deliberately not an
                    // infinite sentinel -- that would poison the first call's `elapsed` value
                    // into +inf once `dt` is added to it below): this guarantees the first
                    // evaluate_blocks call is always "due" (time_since_sample + dt >= ts
                    // trivially), while keeping `elapsed` a small, finite, sane first-call
                    // value (one sample period, plus that first step's own dt) instead of
                    // corrupting the block's own state with an infinite integration step.
                    time_since_sample: sample_time.unwrap_or(0.0),
                    last_output: vec![0.0; output_names.len()],
                }
            }
            _ => BlockState::Stateless,
        };
        block_states.push(state);
    }

    let mut prev_diode_raw_ioff: BTreeMap<String, f64> = BTreeMap::new();
    let mut prev_segments: Option<Vec<Segment>> = None;
    let mut prev_gate_states: Option<BTreeMap<String, GateState>> = None;
    let mut ringing_cooldown: u32 = 0;
    // Empty on the first step, matching every dynamic block's own "starts at rest" convention
    // -- any Signal::BlockPrev reference resolves to 0.0 before any block has ever produced an
    // output.
    let mut prev_outputs: BTreeMap<String, f64> = BTreeMap::new();

    let mut trace = Vec::new();
    let mut t = 0.0;

    match step {
        TimeStep::Fixed(dt) => {
            let steps = (t_final / dt).round() as usize;
            trace.reserve(steps);
            for step_index in 0..steps {
                t += dt;

                let outputs = evaluate_blocks(
                    blocks,
                    &order,
                    &mut block_states,
                    &point_prev,
                    &prev_outputs,
                    t,
                    dt,
                )?;
                let gate_states = resolve_gates(gates, &outputs);
                let states: BTreeMap<String, (Mosfet, GateState)> = mosfets
                    .iter()
                    .map(|(name, m)| {
                        (
                            name.clone(),
                            (*m, *gate_states.get(name).unwrap_or(&GateState::Off)),
                        )
                    })
                    .collect();
                let gate_changed = prev_gate_states.as_ref() != Some(&gate_states);
                let forced = step_index == 0 || gate_changed || ringing_cooldown > 0;

                let (system, all_diodes) =
                    crate::build_with_mosfets(source, dialect, diodes, &states, shared_r_on)?;
                let (point, used_backward_euler) = step_with_fallback(
                    &system,
                    source,
                    dialect,
                    &all_diodes,
                    x_prev_prev.as_deref(),
                    &x_prev,
                    dt,
                    &prev_diode_raw_ioff,
                    &prev_segments,
                    forced,
                    t,
                    &outputs,
                    &prev_outputs,
                )?;
                if used_backward_euler && !forced {
                    ringing_cooldown = RINGING_COOLDOWN_STEPS;
                } else if ringing_cooldown > 0 {
                    ringing_cooldown = ringing_cooldown.saturating_sub(1);
                }

                prev_segments = Some(classify_segments(&point));
                prev_gate_states = Some(gate_states);
                prev_diode_raw_ioff = point
                    .diode_names
                    .iter()
                    .cloned()
                    .zip(point.diode_raw_ioff.iter().copied())
                    .collect();
                x_prev_prev = Some(std::mem::replace(&mut x_prev, point.x.clone()));
                point_prev = point.clone();
                prev_outputs = outputs.clone();
                trace.push((t, point, outputs));
            }
        }
        TimeStep::Adaptive(config) => {
            let mut dt_next = config.dt_init;
            let mut step_index = 0usize;
            while t < t_final {
                let mut dt = dt_next.min(t_final - t);
                loop {
                    let mut trial_block_states = block_states.clone();
                    let t_candidate = t + dt;
                    let outputs = evaluate_blocks(
                        blocks,
                        &order,
                        &mut trial_block_states,
                        &point_prev,
                        &prev_outputs,
                        t_candidate,
                        dt,
                    )?;
                    let gate_states = resolve_gates(gates, &outputs);
                    let states: BTreeMap<String, (Mosfet, GateState)> = mosfets
                        .iter()
                        .map(|(name, m)| {
                            (
                                name.clone(),
                                (*m, *gate_states.get(name).unwrap_or(&GateState::Off)),
                            )
                        })
                        .collect();
                    let gate_changed = prev_gate_states.as_ref() != Some(&gate_states);
                    let forced = step_index == 0 || gate_changed || ringing_cooldown > 0;

                    let (system, all_diodes) =
                        crate::build_with_mosfets(source, dialect, diodes, &states, shared_r_on)?;
                    let attempt = step_control::lte_attempt(
                        &system,
                        source,
                        dialect,
                        &all_diodes,
                        x_prev_prev.as_deref(),
                        &x_prev,
                        dt,
                        &prev_diode_raw_ioff,
                        &prev_segments,
                        forced,
                        &config,
                        t,
                        &outputs,
                        &prev_outputs,
                    )?;

                    if !attempt.accept {
                        dt = attempt.suggested_dt_next;
                        continue;
                    }

                    if attempt.used_backward_euler && !forced {
                        ringing_cooldown = RINGING_COOLDOWN_STEPS;
                    } else if ringing_cooldown > 0 {
                        ringing_cooldown = ringing_cooldown.saturating_sub(1);
                    }

                    block_states = trial_block_states;
                    t = t_candidate;
                    dt_next = attempt.suggested_dt_next;
                    prev_outputs = outputs.clone();
                    prev_segments = Some(classify_segments(&attempt.point));
                    prev_gate_states = Some(gate_states);
                    prev_diode_raw_ioff = attempt
                        .point
                        .diode_names
                        .iter()
                        .cloned()
                        .zip(attempt.point.diode_raw_ioff.iter().copied())
                        .collect();
                    x_prev_prev = Some(std::mem::replace(&mut x_prev, attempt.point.x.clone()));
                    point_prev = attempt.point.clone();
                    trace.push((t, attempt.point, outputs));
                    step_index += 1;
                    break;
                }
            }
        }
    }
    Ok(trace)
}
