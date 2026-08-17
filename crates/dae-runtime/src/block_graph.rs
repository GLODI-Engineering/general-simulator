//! A closed loop assembled from named, independently reusable `continuous-blocks` blocks
//! (`Const`, `Pwl`, `Sum`, `Gain`, `Pid`, `Vco`) wired together by the caller — the same
//! discipline a real block-diagram tool (a reference tool, a reference tool) uses: an error signal is a `Sum`
//! block's output, a frequency-modulated PWM carrier is `Pid -> Gain -> Vco`, not a single
//! function that bakes a specific topology together. [`crate::simulate_closed_loop`] (a
//! single fixed `Sum`-then-`Pid`-then-duty-comparator topology) remains for the common
//! duty-modulated case; this module is for anything else, including frequency modulation,
//! without needing a new `dae-runtime` function for every new topology.
//!
//! Same sampled-data co-simulation model as [`crate::simulate_closed_loop`]: every block
//! reads the *previous* circuit step's measurement, the whole graph is evaluated once per
//! circuit step (in declaration order — see [`BlockInstance`]), and the result decides gate
//! states before the circuit step itself is solved.

use std::collections::BTreeMap;

use continuous_blocks::{math_ops, Pid, Vco};
use pwl_devices::{Diode, Mosfet};
use spice_core::Dialect;

use crate::{
    classify_segments, step_with_fallback, DaeError, GateState, OperatingPoint, Segment,
    RINGING_COOLDOWN_STEPS,
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
/// (recomputed fresh from their inputs every step); `Pid`/`Vco` carry their own state forward
/// across steps (a compiled [`Pid`]'s `StateSpace`, and a [`Vco`]'s phase, respectively).
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
    /// block instead of baked into the whole closed-loop function. `clamp` is this PID's own
    /// notion of "my output is saturated," independent of whatever downstream `Gain`/`Vco`
    /// blocks do to it after — same as a real PID block's own configured output limits.
    Pid { pid: Pid, clamp: (f64, f64) },
    /// A voltage-controlled oscillator (see [`Vco`]).
    Vco(Vco),
}

/// One named block instance and where its inputs (if any) come from. `Const`/`Pwl` blocks
/// must have zero inputs; `Sum` needs one input per sign; `Gain`/`Pid`/`Vco` each need exactly
/// one. Evaluated in the order given in the slice passed to [`simulate_closed_loop_blocks`] —
/// every input must reference a `Measure` or a block *earlier* in that same slice (source
/// blocks, naturally, need none).
#[derive(Debug, Clone, PartialEq)]
pub struct BlockInstance {
    pub name: String,
    pub kind: BlockKind,
    pub inputs: Vec<Signal>,
}

/// How one MOSFET's gate state is derived from a named [`BlockKind::Vco`] block's current
/// output (a ramp in `[0, 1)`): on while `(ramp + phase).rem_euclid(1.0) < duty` — see
/// [`math_ops::pwm_from_ramp`]. Several `GateBinding`s naming the same `vco` share one
/// oscillator with different phase offsets (a half-bridge's two complementary switches, for
/// instance), rather than needing one `Vco` block per gate.
#[derive(Debug, Clone, PartialEq)]
pub struct GateBinding {
    pub vco: String,
    pub phase: f64,
    pub duty: f64,
}

enum BlockState {
    Stateless,
    Pid {
        state_space: continuous_blocks::StateSpace,
        x: Vec<f64>,
    },
    Vco {
        vco: Vco,
        phase: f64,
    },
}

/// One step's result: `(t, OperatingPoint, block_outputs)`, where `block_outputs` is every
/// block's value that step (by name) — useful for plotting a controller's internal signals
/// (e.g. a `Vco`'s commanded frequency) without needing to separately re-derive them.
pub type ClosedLoopBlocksStep = (f64, OperatingPoint, BTreeMap<String, f64>);

/// Runs a closed-loop transient driven by a graph of [`BlockInstance`]s instead of a single
/// fixed controller — see this module's doc comment.
#[allow(clippy::too_many_arguments)]
pub fn simulate_closed_loop_blocks(
    source: &str,
    dialect: Dialect,
    diodes: &BTreeMap<String, Diode>,
    mosfets: &BTreeMap<String, Mosfet>,
    blocks: &[BlockInstance],
    gates: &BTreeMap<String, GateBinding>,
    shared_r_on: f64,
    x_initial: Option<&[f64]>,
    t_final: f64,
    dt: f64,
) -> Result<Vec<ClosedLoopBlocksStep>, DaeError> {
    let block_names: std::collections::BTreeSet<&str> =
        blocks.iter().map(|b| b.name.as_str()).collect();
    for binding in gates.values() {
        if !block_names.contains(binding.vco.as_str()) {
            return Err(DaeError::UnknownBlockInput(binding.vco.clone()));
        }
    }

    let initial_states: BTreeMap<String, (Mosfet, GateState)> = mosfets
        .iter()
        .map(|(name, m)| (name.clone(), (*m, GateState::Off)))
        .collect();
    let (system0, all_diodes0) =
        crate::build_with_mosfets(source, dialect, diodes, &initial_states, shared_r_on)?;
    let mut x_prev = match x_initial {
        Some(x) => x.to_vec(),
        None => vec![0.0; system0.order()],
    };
    let (mut point_prev, _) = step_with_fallback(
        &system0,
        source,
        dialect,
        &all_diodes0,
        None,
        &x_prev,
        dt,
        &BTreeMap::new(),
        &None,
        true,
    )?;
    let mut x_prev_prev: Option<Vec<f64>> =
        Some(std::mem::replace(&mut x_prev, point_prev.x.clone()));

    let mut block_states: Vec<BlockState> = blocks
        .iter()
        .map(|b| match &b.kind {
            BlockKind::Pid { pid, .. } => {
                let state_space = pid.to_state_space();
                let x = vec![0.0; state_space.states()];
                BlockState::Pid { state_space, x }
            }
            BlockKind::Vco(vco) => BlockState::Vco {
                vco: *vco,
                phase: 0.0,
            },
            _ => BlockState::Stateless,
        })
        .collect();

    let mut prev_diode_raw_ioff: BTreeMap<String, f64> = BTreeMap::new();
    let mut prev_segments: Option<Vec<Segment>> = None;
    let mut prev_gate_states: Option<BTreeMap<String, GateState>> = None;
    let mut ringing_cooldown: u32 = 0;

    let steps = (t_final / dt).round() as usize;
    let mut trace = Vec::with_capacity(steps);
    let mut t = 0.0;
    for step_index in 0..steps {
        t += dt;

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
                (BlockKind::Pid { clamp, .. }, BlockState::Pid { state_space, x }) => {
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
                (BlockKind::Vco(_), BlockState::Vco { vco, phase }) => {
                    *phase = vco.step(*phase, input_vals[0], dt);
                    *phase
                }
                _ => unreachable!("BlockState variant always matches its BlockKind"),
            };
            outputs.insert(block.name.clone(), value);
        }

        let gate_states: BTreeMap<String, GateState> = gates
            .iter()
            .map(|(mosfet_name, binding)| {
                let ramp = outputs
                    .get(&binding.vco)
                    .copied()
                    .expect("validated at function entry");
                let state = if math_ops::pwm_from_ramp(ramp, binding.phase, binding.duty) {
                    GateState::On
                } else {
                    GateState::Off
                };
                (mosfet_name.clone(), state)
            })
            .collect();
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
    Ok(trace)
}
