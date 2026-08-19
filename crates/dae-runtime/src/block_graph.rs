//! Resolves every MOSFET's gate state, each transient step, from a graph of named,
//! independently reusable `continuous-blocks` blocks (`Const`, `Pwl`, `Sum`, `Gain`, `Pid`,
//! `StateSpace`, `TransferFunction`, `Vco`, `Hysteresis`) wired together by the caller — the same discipline
//! a real block-diagram tool (a reference tool, a reference tool) uses: an error signal is a `Sum` block's
//! output, a filtered-derivative PID compensator is a `TransferFunction` given its own
//! `N(s)/D(s)` coefficients, and a frequency-modulated PWM carrier is `Pid -> Gain -> Vco`, not
//! a single function that bakes a specific topology together.
//!
//! **There is no separate "closed-loop mode."** A [`GateBinding`] can be a plain fixed state or
//! fixed-frequency/fixed-duty PWM (no block involved at all — the historically "open-loop"
//! case) just as easily as a block whose input chain happens to trace back to a
//! [`Signal::Measure`] of the circuit's own state (the historically "closed-loop" case) — both
//! are resolved by exactly the same code, every step, because from the solver's point of view
//! they're the same kind of question: "what's this device's terminal condition right now,"
//! answered from whatever the netlist and device file actually say. Real circuit simulators
//! (SPICE, a reference tool) don't have a closed-loop *mode* either — closed-loop is a property of how a
//! circuit happens to be wired, not an analysis type the tool needs to be told about upfront;
//! `.op` and `.tran` are genuinely distinct analyses (different equations solved), but nothing
//! about *this* function is analysis-specific.
//!
//! Sampled-data co-simulation: every block reads the *previous* circuit step's measurement,
//! the whole graph is evaluated once per circuit step (in declaration order — see
//! [`BlockInstance`]), and the result decides gate states before the circuit step itself is
//! solved. [`crate::simulate_closed_loop`] (a single fixed `Sum`-then-`Pid`-then-duty-
//! comparator topology, named for the one case it was first built for) remains as a lighter
//! Rust-level convenience for simple direct callers; this module is the general one, used by
//! `elspice-pwl-cli` unconditionally for every MOSFET-containing transient run.

use std::collections::BTreeMap;

use continuous_blocks::{
    math_ops, Hysteresis, MathFn1, MathFn2, MathFn3, Pid, StateSpace, TransferFunction, Vco,
};
use pwl_devices::{Diode, Mosfet};
use spice_core::Dialect;

use crate::{
    classify_segments, sawtooth_carrier, step_control, step_with_fallback, DaeError, GateState,
    OperatingPoint, Segment, TimeStep, RINGING_COOLDOWN_STEPS,
};

/// Where a block's input value comes from: another block's output this same step, or the
/// circuit's own previous-step operating point (`V(node)` or `I(branch)`, anything
/// [`OperatingPoint::value`] accepts).
#[derive(Debug, Clone, PartialEq)]
pub enum Signal {
    Block(String),
    Measure(String),
}

/// One block's behavior. `Const`/`Pwl` are sources (zero inputs); `Sum`/`Gain` are stateless
/// (recomputed fresh from their inputs every step); `Pid`/`StateSpace`/`TransferFunction`/`Vco`
/// carry their own state forward across steps.
#[derive(Debug, Clone, PartialEq)]
pub enum BlockKind {
    /// A fixed value, ignoring time — e.g. a nominal frequency or a fixed setpoint.
    Const(f64),
    /// A piecewise-constant function of time: the value from the last point at or before `t`
    /// (the first point's value for `t` before it). Used for reference schedules, including
    /// step tests (two points is a step at the second point's time).
    Pwl(Vec<(f64, f64)>),
    /// Weighted sum of its inputs, one sign per input (`+1.0`/`-1.0` for an error junction).
    Sum(Vec<f64>),
    /// Scales its single input.
    Gain(f64),
    /// A compiled PID with two-sided conditional-integration anti-windup against
    /// `clamp = (lo, hi)` — see [`crate::simulate_closed_loop`]'s doc comment for why
    /// two-sided anti-windup matters; the mechanism here is identical, just attached to this
    /// block instead of baked into a whole controller function. `clamp` is this PID's own
    /// notion of "my output is saturated," independent of whatever downstream `Gain`/`Vco`
    /// blocks do to it after — same as a real PID block's own configured output limits.
    Pid { pid: Pid, clamp: (f64, f64) },
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
    /// A voltage-controlled oscillator (see [`Vco`]).
    Vco(Vco),
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
    /// `Pid` feeding a [`GateBinding::Pwm`]). Its output is `1.0`/`0.0`, read directly by a
    /// [`GateBinding::Block`] rather than compared against a carrier.
    Hysteresis(Hysteresis),
}

/// One named block instance and where its inputs (if any) come from. `Const`/`Pwl` blocks
/// must have zero inputs; `Sum`/`Product` need one input per sign/factor; `Gain`/`Pid`/
/// `StateSpace`/`TransferFunction`/`Vco`/`Saturation`/`Table`/`MathFn1` each need exactly one;
/// `MathFn2` needs two; `MathFn3` needs three. Evaluated in the order given in the slice
/// passed to [`simulate_transient_with_blocks`] — every input must reference a `Measure` or a
/// block *earlier* in that same slice (source blocks, naturally, need none).
#[derive(Debug, Clone, PartialEq)]
pub struct BlockInstance {
    pub name: String,
    pub kind: BlockKind,
    pub inputs: Vec<Signal>,
}

/// How one MOSFET's gate state is resolved, every step. The first two variants need no block
/// graph at all (the historically "open-loop" cases); the last two read a named block's
/// current output (which may or may not itself depend on a circuit measurement somewhere
/// upstream — this type doesn't need to know or care which).
#[derive(Debug, Clone, PartialEq)]
pub enum GateBinding {
    /// Always the same state.
    Fixed(GateState),
    /// Fixed-frequency, fixed-duty PWM: on while [`sawtooth_carrier`]`(t, freq_hz) < duty`.
    PwmFixed { freq_hz: f64, duty: f64 },
    /// Frequency modulation: on while `(ramp + phase).rem_euclid(1.0) < duty`, where `ramp`
    /// is a named [`BlockKind::Vco`] block's current `[0, 1)` output — see
    /// [`math_ops::pwm_from_ramp`]. Several `GateBinding::Vco`s naming the same block share one
    /// oscillator with different phase offsets (a half-bridge's two complementary switches),
    /// rather than needing one `Vco` block per gate.
    Vco { vco: String, phase: f64, duty: f64 },
    /// Duty modulation at a fixed carrier frequency: on while
    /// [`sawtooth_carrier`]`(t, freq_hz)` is below a named block's current output (clamped to
    /// `[0, 1]`) — the standard buck/boost-style comparator, with the duty *command* coming
    /// from anywhere in the graph (a `Pid`, a filtered `TransferFunction`, ...) instead of
    /// being a fixed value.
    Pwm { duty: String, freq_hz: f64 },
    /// Direct duty-less on/off control: on while a named block's current output is `>= 0.5` —
    /// no carrier at all, since bang-bang/hysteresis control has no fixed switching frequency
    /// to compare against (unlike [`GateBinding::Pwm`]). Meant for a [`BlockKind::Hysteresis`]
    /// block, whose output is already `1.0`/`0.0`, but works with any block.
    Block(String),
}

impl GateBinding {
    /// The block this binding reads from, if any (`Fixed`/`PwmFixed` need none).
    fn source_block(&self) -> Option<&str> {
        match self {
            GateBinding::Fixed(_) | GateBinding::PwmFixed { .. } => None,
            GateBinding::Vco { vco, .. } => Some(vco),
            GateBinding::Pwm { duty, .. } => Some(duty),
            GateBinding::Block(name) => Some(name),
        }
    }

    fn resolve(&self, t: f64, outputs: &BTreeMap<String, f64>) -> GateState {
        let on = match self {
            GateBinding::Fixed(state) => return *state,
            GateBinding::PwmFixed { freq_hz, duty } => sawtooth_carrier(t, *freq_hz) < *duty,
            GateBinding::Vco { vco, phase, duty } => {
                let ramp = outputs[vco.as_str()];
                math_ops::pwm_from_ramp(ramp, *phase, *duty)
            }
            GateBinding::Pwm { duty, freq_hz } => {
                let source = outputs[duty.as_str()];
                sawtooth_carrier(t, *freq_hz) < source.clamp(0.0, 1.0)
            }
            GateBinding::Block(name) => outputs[name.as_str()] >= 0.5,
        };
        if on {
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
    Hysteresis {
        hysteresis: Hysteresis,
        on: bool,
    },
}

/// One step's result: `(t, OperatingPoint, block_outputs)`, where `block_outputs` is every
/// block's value that step (by name) — useful for plotting a controller's internal signals
/// (e.g. a `Vco`'s commanded frequency) without needing to separately re-derive them. Empty if
/// the run used no blocks at all (every gate `Fixed`/`PwmFixed`).
pub type TransientWithBlocksStep = (f64, OperatingPoint, BTreeMap<String, f64>);

/// Evaluates every block once (in declaration order), advancing `block_states` in place at
/// step size `dt` and reading any [`Signal::Measure`] from `point_prev` — the one piece of
/// per-step work both [`TimeStep::Fixed`] and [`TimeStep::Adaptive`] need identically, factored
/// out so the adaptive loop below can re-run it (against a *cloned* `block_states`) once per
/// retry at a shrinking trial `dt`, without duplicating the block-dispatch match arms.
fn evaluate_blocks(
    blocks: &[BlockInstance],
    block_states: &mut [BlockState],
    point_prev: &OperatingPoint,
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
            Signal::Measure(node) => Ok(point_prev
                .value(&format!("V({node})"))
                .unwrap_or_else(|| point_prev.value(node).unwrap_or(0.0))),
        }
    };

    for (block, state) in blocks.iter().zip(block_states.iter_mut()) {
        let input_vals: Vec<f64> = block
            .inputs
            .iter()
            .map(|s| resolve(&outputs, s))
            .collect::<Result<_, _>>()?;

        let value = match (&block.kind, state) {
            (BlockKind::Const(v), _) => *v,
            (BlockKind::Pwl(points), _) => {
                let mut v = points.first().map(|(_, v)| *v).unwrap_or(0.0);
                for &(t_i, v_i) in points {
                    if t_i <= t {
                        v = v_i;
                    } else {
                        break;
                    }
                }
                v
            }
            (BlockKind::Sum(signs), _) => math_ops::sum(&input_vals, signs),
            (BlockKind::Gain(k), _) => math_ops::gain(*k, input_vals[0]),
            (BlockKind::Product, _) => math_ops::product(&input_vals),
            (BlockKind::Saturation(limit), _) => math_ops::saturation(input_vals[0], *limit),
            (BlockKind::Table(points), _) => {
                continuous_blocks::waveform_arithmetic::table(input_vals[0], points)
            }
            (BlockKind::MathFn1(f), _) => f.call(input_vals[0]),
            (BlockKind::MathFn2(f), _) => f.call(input_vals[0], input_vals[1]),
            (BlockKind::MathFn3(f), _) => f.call(input_vals[0], input_vals[1], input_vals[2]),
            (BlockKind::Pid { clamp, .. }, BlockState::Dynamic { state_space, x }) => {
                let (lo, hi) = *clamp;
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
            (BlockKind::Hysteresis(_), BlockState::Hysteresis { hysteresis, on }) => {
                *on = hysteresis.step(*on, input_vals[0]);
                if *on {
                    1.0
                } else {
                    0.0
                }
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
    t: f64,
) -> BTreeMap<String, GateState> {
    gates
        .iter()
        .map(|(mosfet_name, binding)| (mosfet_name.clone(), binding.resolve(t, outputs)))
        .collect()
}

/// Runs a transient with every MOSFET's gate resolved from a [`GateBinding`] each step — see
/// this module's doc comment for why there's no separate "closed-loop" entry point: a
/// `GateBinding::Fixed`/`PwmFixed` device and a `GateBinding::Vco`/`Pwm` device driven by a
/// `Sum`-`Pid`-`Vco` chain that happens to read [`Signal::Measure`] are resolved by exactly the
/// same loop below. `blocks` may be empty if every gate is `Fixed`/`PwmFixed`. `step` picks
/// fixed or adaptive timing — see [`TimeStep`]/[`AdaptiveConfig`]. Adaptive mode here can't
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
    let block_names: std::collections::BTreeSet<&str> =
        blocks.iter().map(|b| b.name.as_str()).collect();
    for binding in gates.values() {
        if let Some(needed) = binding.source_block() {
            if !block_names.contains(needed) {
                return Err(DaeError::UnknownBlockInput(needed.to_string()));
            }
        }
    }

    // Only used to learn the system's `unknowns` ordering/count for the pre-first-step
    // `point_prev` below and the default `x_initial` — no step is solved with it. Solving one
    // would perturb `x_prev` away from `x_initial` before the real first step even runs, a
    // real behavioral difference from `simulate_transient_with_mosfets` for the plain
    // fixed/PWM case (caught by exactly that mismatch: this function must reduce to identical
    // numbers as that one whenever no block reads a `Signal::Measure`).
    let initial_states: BTreeMap<String, (Mosfet, GateState)> = mosfets
        .iter()
        .map(|(name, m)| (name.clone(), (*m, GateState::Off)))
        .collect();
    let (system0, _) =
        crate::build_with_mosfets(source, dialect, diodes, &initial_states, shared_r_on)?;
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

    let mut block_states: Vec<BlockState> = blocks
        .iter()
        .map(|b| {
            let dynamic = |state_space: StateSpace| {
                let x = vec![0.0; state_space.states()];
                BlockState::Dynamic { state_space, x }
            };
            match &b.kind {
                BlockKind::Pid { pid, .. } => dynamic(pid.to_state_space()),
                BlockKind::StateSpace(ss) => dynamic(ss.clone()),
                BlockKind::TransferFunction(tf) => dynamic(tf.to_state_space()),
                BlockKind::Vco(vco) => BlockState::Vco {
                    vco: *vco,
                    phase: 0.0,
                },
                BlockKind::Hysteresis(hysteresis) => BlockState::Hysteresis {
                    hysteresis: *hysteresis,
                    on: false,
                },
                _ => BlockState::Stateless,
            }
        })
        .collect();

    let mut prev_diode_raw_ioff: BTreeMap<String, f64> = BTreeMap::new();
    let mut prev_segments: Option<Vec<Segment>> = None;
    let mut prev_gate_states: Option<BTreeMap<String, GateState>> = None;
    let mut ringing_cooldown: u32 = 0;

    let mut trace = Vec::new();
    let mut t = 0.0;

    match step {
        TimeStep::Fixed(dt) => {
            let steps = (t_final / dt).round() as usize;
            trace.reserve(steps);
            for step_index in 0..steps {
                t += dt;

                let outputs = evaluate_blocks(blocks, &mut block_states, &point_prev, t, dt)?;
                let gate_states = resolve_gates(gates, &outputs, t);
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
                        &mut trial_block_states,
                        &point_prev,
                        t_candidate,
                        dt,
                    )?;
                    let gate_states = resolve_gates(gates, &outputs, t_candidate);
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
