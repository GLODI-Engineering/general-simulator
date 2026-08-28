//! Evaluates a graph of `general_mna::block_graph` types (`BlockKind`/`BlockInstance`/`Signal`/
//! `GateBinding` — `Const`, `Time`, `Pwc`, `Pwl`, `Sin`, `Pulse`, `Exp`, `Sffm`, `Sum`, `Gain`,
//! `Pid`, `StateSpace`, `TransferFunction`, `Vco`, `Pwm`, `PhaseShiftPwm`, `Product`,
//! `Saturation`, `Table`, `MathFn1`/`2`/`3`, `Hysteresis`, `CoordinateTransform`, `Pmsm`,
//! `CScript`, `Probe`, `Sig2Voltage`, `Sig2Current`), resolving every MOSFET's gate
//! state each transient step. `general-mna` owns *what these types are* and *parsing them out
//! of source text* (`general_mna::build_system`); this module owns *evaluating* the graph over
//! time (`BlockState`, `evaluate_blocks`, `topological_order`,
//! `simulate_transient_with_blocks`) — an error signal is a `Sum` block's output, a filtered-
//! derivative PID compensator is a `TransferFunction` given its own `N(s)/D(s)` coefficients,
//! and a frequency-modulated PWM carrier is `Pid -> Gain -> PhaseShiftPwm`, not a single
//! function that bakes a specific topology together.
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
//! `general-simulator-cli` unconditionally for every MOSFET-containing transient run.

use std::collections::BTreeMap;

use continuous_blocks::{math_ops, Hysteresis, Pmsm, StateSpace, Vco};
use cscript_ffi::CScriptRegistry;
use general_spice_core::Dialect;
use pwl_devices::{Diode, Mosfet};

use crate::{
    classify_segments, sawtooth_carrier, step_control, step_with_fallback, DaeError, GateState,
    OperatingPoint, Segment, TimeStep, RINGING_COOLDOWN_STEPS,
};

use general_mna::block_graph::block_kind_name;
pub use general_mna::block_graph::{
    BlockInstance, BlockKind, ConstValue, GainValue, GateBinding, PidClamp, ProbeTarget, Signal,
    SignalValue,
};

/// Every input must be `Scalar` — the default rule for a `BlockKind` that has no elementwise/
/// broadcast/flattening rule of its own (`Pid`, `Vco`, `Hysteresis`, `TransferFunction`, `Pwm`,
/// `PhaseShiftPwm`, `Sig2Voltage`, `Sig2Current`). Returns the plain `f64` values in
/// order, or a clear error naming the block the moment any input is a `Vector`.
fn require_all_scalar(block_name: &str, input_vals: &[SignalValue]) -> Result<Vec<f64>, DaeError> {
    input_vals
        .iter()
        .map(|v| {
            v.as_scalar()
                .ok_or_else(|| DaeError::VectorSignalNotSupported {
                    block: block_name.to_string(),
                })
        })
        .collect()
}

/// Flattens `input_vals` (each independently `Scalar` or `Vector`) into one `Vec<f64>`, in
/// order — the input-bundling convention `StateSpace`, `CoordinateTransform`, `Pmsm`, and
/// `cscript` all share: a downstream block wanting several upstream values can list them one
/// per `Signal`, mixing scalar and vector references freely, and this is where they get
/// concatenated into the flat vector each of those blocks' own math actually consumes.
fn flatten(input_vals: &[SignalValue]) -> Vec<f64> {
    input_vals
        .iter()
        .flat_map(|v| v.as_slice().iter().copied())
        .collect()
}

/// The common vector length among `operands` (every `Vector` operand must share one length; a
/// lone `Scalar` operand broadcasts against it) — `None` if every operand is `Scalar` (no
/// broadcasting needed, the block should just compute its ordinary scalar result). Used by
/// `MathFn2`/`MathFn3`, which — unlike `Sum`/`Product` — always have an unambiguous pairing for
/// a lone scalar operand, so broadcasting it is the right default rather than an error.
fn common_vector_len(
    block_name: &str,
    operands: &[&SignalValue],
) -> Result<Option<usize>, DaeError> {
    let mut common: Option<usize> = None;
    for v in operands {
        if let SignalValue::Vector(xs) = v {
            match common {
                None => common = Some(xs.len()),
                Some(n) if n == xs.len() => {}
                Some(n) => {
                    return Err(DaeError::VectorSignalSizeMismatch {
                        block: block_name.to_string(),
                        expected: n,
                        got: xs.len(),
                    })
                }
            }
        }
    }
    Ok(common)
}

/// `v`'s own value at element `i` — its single scalar if `v` is `Scalar` (the broadcast case),
/// or `v`'s own `i`-th element if `v` is `Vector`. Panics if `v` is a `Vector` shorter than
/// `i + 1`; callers only ever index up to the common length `common_vector_len` already
/// validated every `Vector` operand agrees with, so this is never actually out of bounds.
fn broadcast_at(v: &SignalValue, i: usize) -> f64 {
    match v {
        SignalValue::Scalar(x) => *x,
        SignalValue::Vector(xs) => xs[i],
    }
}

/// Every input must be a `Vector`, all the same length — `Sum`/`Product`'s own rule, distinct
/// from `common_vector_len`'s broadcast: an N-input reduction has no unambiguous placement for
/// a lone scalar once more than one vector is already present, so a mix of `Scalar` and
/// `Vector` inputs is rejected outright rather than guessed at (see
/// `book/dev-guide/src/vector-signals.md`, category 3).
fn require_uniform_vectors(
    block_name: &str,
    input_vals: &[SignalValue],
) -> Result<usize, DaeError> {
    let mut common: Option<usize> = None;
    for v in input_vals {
        let SignalValue::Vector(xs) = v else {
            return Err(DaeError::VectorSignalNotSupported {
                block: block_name.to_string(),
            });
        };
        match common {
            None => common = Some(xs.len()),
            Some(n) if n == xs.len() => {}
            Some(n) => {
                return Err(DaeError::VectorSignalSizeMismatch {
                    block: block_name.to_string(),
                    expected: n,
                    got: xs.len(),
                })
            }
        }
    }
    Ok(common.unwrap_or(0))
}

/// Filters a block-graph output map down to its `Scalar` entries only, discarding any `Vector`
/// ones — used specifically where a `V`/`I` source's own literal value needs a scalar-only
/// symbol table (`fold_and_solve`'s `extra_values`/`extra_values_prev`): the *only* symbols
/// that can legitimately appear there are a `Sig2Voltage`/`Sig2Current` converter's own name,
/// and both are guaranteed `Scalar` (they reject a `Vector` input at evaluation time — see
/// their own `evaluate_blocks` arm), so dropping any `Vector` entry here is safe, not a silent
/// bug: nothing that could legitimately need one is ever filtered out.
fn scalar_only(outputs: &BTreeMap<String, SignalValue>) -> BTreeMap<String, f64> {
    outputs
        .iter()
        .filter_map(|(k, v)| v.as_scalar().map(|x| (k.clone(), x)))
        .collect()
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
    /// every step the block *doesn't* actually run its output function. Cloning this variant
    /// (only ever needed by [`TimeStep::Adaptive`]'s retry loop, which clones the whole
    /// `block_states` vector before every trial) calls into `cscript_clone` for `instance` and
    /// **panics** if the library doesn't export it — see
    /// [`simulate_transient_with_blocks`]'s own upfront check, which exists specifically so
    /// that panic is unreachable in practice. `xc` is the block's own continuous-state vector
    /// (empty when `xc_count == 0`) — a plain, RK4-cloneable `Vec<f64>`, not part of the opaque
    /// `void *state` `instance` owns; see `cscript_ffi`'s own module doc comment, "The optional
    /// continuous-state (`xc`) contract."
    CScript {
        instance: cscript_ffi::CScriptInstance,
        time_since_sample: f64,
        last_output: Vec<f64>,
        xc: Vec<f64>,
    },
    /// The Python-hosted counterpart to `CScript` above — same zero-order-hold `sample_time`
    /// bookkeeping, same `xc` continuous-state vector, `instance` a live
    /// [`pyblock_ffi::PyBlockInstance`] instead of a C one. Cloning this variant uses Python's
    /// own generic `copy.deepcopy` (see `pyblock_ffi`'s own module doc comment) — unlike
    /// `CScript`, there is no "doesn't support clone" panic path, since `deepcopy` needs no
    /// author opt-in.
    PyBlock {
        instance: pyblock_ffi::PyBlockInstance,
        time_since_sample: f64,
        last_output: Vec<f64>,
        xc: Vec<f64>,
    },
    /// The counterpart to `PyBlock` above for [`BlockKind::PyFunction`] — a genuinely separate
    /// variant (not reusing `PyBlock`'s own fields), since a stateless
    /// [`pyblock_ffi::PyFunctionInstance`] has no `xc` and cloning it is always trivially cheap
    /// (`clone_ref`, never `copy.deepcopy`, never fallible) — see that type's own doc comment.
    /// Still needs the same zero-order-hold `sample_time` bookkeeping every other zero-order-
    /// hold block has.
    PyFunction {
        instance: pyblock_ffi::PyFunctionInstance,
        time_since_sample: f64,
        last_output: Vec<f64>,
    },
}

/// One step's result: `(t, OperatingPoint, block_outputs)`, where `block_outputs` is every
/// block's value that step (by name) — useful for plotting a controller's internal signals
/// (e.g. a `Vco`'s commanded frequency) without needing to separately re-derive them.
pub type TransientWithBlocksStep = (f64, OperatingPoint, BTreeMap<String, SignalValue>);

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
            | BlockKind::PyBlock { output_names, .. }
            | BlockKind::PyFunction { output_names, .. }
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
/// neither `general-mna`'s own electrical-domain `PWL(...)` source nor any prior signal-domain
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
    outputs: &mut BTreeMap<String, SignalValue>,
    output_names: &[String],
    main: bool,
    complement: bool,
) -> f64 {
    if let Some(name) = output_names.get(1) {
        outputs.insert(
            name.clone(),
            SignalValue::Scalar(if complement { 1.0 } else { 0.0 }),
        );
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
    prev_outputs: &BTreeMap<String, SignalValue>,
    t: f64,
    dt: f64,
) -> Result<BTreeMap<String, SignalValue>, DaeError> {
    let mut outputs: BTreeMap<String, SignalValue> = BTreeMap::new();
    let resolve = |outputs: &BTreeMap<String, SignalValue>,
                   signal: &Signal|
     -> Result<SignalValue, DaeError> {
        match signal {
            Signal::Block(name) => outputs
                .get(name)
                .cloned()
                .ok_or_else(|| DaeError::UnknownBlockInput(name.clone())),
            Signal::BlockPrev(name) => Ok(prev_outputs
                .get(name)
                .cloned()
                .unwrap_or(SignalValue::Scalar(0.0))),
        }
    };

    for &i in order {
        let block = &blocks[i];
        let state = &mut block_states[i];
        let input_vals: Vec<SignalValue> = block
            .inputs
            .iter()
            .map(|s| resolve(&outputs, s))
            .collect::<Result<_, _>>()?;

        let value: SignalValue = match (&block.kind, state) {
            (BlockKind::Const(v), _) => match v {
                ConstValue::Scalar(x) => SignalValue::Scalar(*x),
                ConstValue::Vector(xs) => SignalValue::Vector(xs.clone()),
            },
            (BlockKind::Time, _) => SignalValue::Scalar(t),
            (BlockKind::Probe(target), _) => SignalValue::Scalar(match target {
                ProbeTarget::Voltage(node) => {
                    point_prev.value(&format!("V({node})")).unwrap_or(0.0)
                }
                ProbeTarget::Current(branch) => {
                    point_prev.value(&format!("I({branch})")).unwrap_or(0.0)
                }
            }),
            (BlockKind::Sig2Voltage | BlockKind::Sig2Current, _) => {
                SignalValue::Scalar(require_all_scalar(&block.name, &input_vals)?[0])
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
                SignalValue::Scalar(v)
            }
            (BlockKind::Pwl { points, repeat }, _) => {
                let t_eval = periodic_time(t, points, *repeat);
                SignalValue::Scalar(match points.as_slice() {
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
                })
            }
            (BlockKind::Waveform(f), _) => SignalValue::Scalar(f.value_at(t)),
            (BlockKind::Sum(signs), _) => {
                if input_vals
                    .iter()
                    .all(|v| matches!(v, SignalValue::Scalar(_)))
                {
                    let scalars = require_all_scalar(&block.name, &input_vals)?;
                    SignalValue::Scalar(math_ops::sum(&scalars, signs))
                } else {
                    let n = require_uniform_vectors(&block.name, &input_vals)?;
                    let out: Vec<f64> = (0..n)
                        .map(|i| {
                            let elems: Vec<f64> =
                                input_vals.iter().map(|v| broadcast_at(v, i)).collect();
                            math_ops::sum(&elems, signs)
                        })
                        .collect();
                    SignalValue::Vector(out)
                }
            }
            (BlockKind::Gain(k), _) => match k {
                GainValue::Scalar(k) => match &input_vals[0] {
                    SignalValue::Scalar(x) => SignalValue::Scalar(math_ops::gain(*k, *x)),
                    SignalValue::Vector(xs) => {
                        SignalValue::Vector(xs.iter().map(|&x| math_ops::gain(*k, x)).collect())
                    }
                },
                GainValue::Matrix(mat) => {
                    let n = mat.first().map_or(0, |row| row.len());
                    let SignalValue::Vector(x) = &input_vals[0] else {
                        return Err(DaeError::VectorSignalNotSupported {
                            block: block.name.clone(),
                        });
                    };
                    if x.len() != n {
                        return Err(DaeError::VectorSignalSizeMismatch {
                            block: block.name.clone(),
                            expected: n,
                            got: x.len(),
                        });
                    }
                    let y: Vec<f64> = mat
                        .iter()
                        .map(|row| row.iter().zip(x).map(|(a, b)| a * b).sum())
                        .collect();
                    SignalValue::Vector(y)
                }
            },
            (BlockKind::Product, _) => {
                if input_vals
                    .iter()
                    .all(|v| matches!(v, SignalValue::Scalar(_)))
                {
                    let scalars = require_all_scalar(&block.name, &input_vals)?;
                    SignalValue::Scalar(math_ops::product(&scalars))
                } else {
                    let n = require_uniform_vectors(&block.name, &input_vals)?;
                    let out: Vec<f64> = (0..n)
                        .map(|i| {
                            let elems: Vec<f64> =
                                input_vals.iter().map(|v| broadcast_at(v, i)).collect();
                            math_ops::product(&elems)
                        })
                        .collect();
                    SignalValue::Vector(out)
                }
            }
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
                let duty = require_all_scalar(&block.name, &input_vals)?[0];
                let (main, complement) = math_ops::complementary_pwm_with_deadtime(
                    theta,
                    duty,
                    red * freq_hz,
                    fed * freq_hz,
                );
                SignalValue::Scalar(emit_complementary_pair(
                    &mut outputs,
                    output_names,
                    main,
                    complement,
                ))
            }
            (BlockKind::Saturation(limit), _) => match &input_vals[0] {
                SignalValue::Scalar(x) => SignalValue::Scalar(math_ops::saturation(*x, *limit)),
                SignalValue::Vector(xs) => SignalValue::Vector(
                    xs.iter()
                        .map(|&x| math_ops::saturation(x, *limit))
                        .collect(),
                ),
            },
            (BlockKind::Table(points), _) => match &input_vals[0] {
                SignalValue::Scalar(x) => {
                    SignalValue::Scalar(continuous_blocks::waveform_arithmetic::table(*x, points))
                }
                SignalValue::Vector(xs) => SignalValue::Vector(
                    xs.iter()
                        .map(|&x| continuous_blocks::waveform_arithmetic::table(x, points))
                        .collect(),
                ),
            },
            (BlockKind::MathFn1(f), _) => match &input_vals[0] {
                SignalValue::Scalar(x) => SignalValue::Scalar(f.call(*x)),
                SignalValue::Vector(xs) => {
                    SignalValue::Vector(xs.iter().map(|&x| f.call(x)).collect())
                }
            },
            (BlockKind::MathFn2(f), _) => {
                match common_vector_len(&block.name, &[&input_vals[0], &input_vals[1]])? {
                    None => SignalValue::Scalar(f.call(
                        input_vals[0].as_scalar().expect("checked Scalar above"),
                        input_vals[1].as_scalar().expect("checked Scalar above"),
                    )),
                    Some(n) => SignalValue::Vector(
                        (0..n)
                            .map(|i| {
                                f.call(
                                    broadcast_at(&input_vals[0], i),
                                    broadcast_at(&input_vals[1], i),
                                )
                            })
                            .collect(),
                    ),
                }
            }
            (BlockKind::MathFn3(f), _) => {
                match common_vector_len(
                    &block.name,
                    &[&input_vals[0], &input_vals[1], &input_vals[2]],
                )? {
                    None => SignalValue::Scalar(f.call(
                        input_vals[0].as_scalar().expect("checked Scalar above"),
                        input_vals[1].as_scalar().expect("checked Scalar above"),
                        input_vals[2].as_scalar().expect("checked Scalar above"),
                    )),
                    Some(n) => SignalValue::Vector(
                        (0..n)
                            .map(|i| {
                                f.call(
                                    broadcast_at(&input_vals[0], i),
                                    broadcast_at(&input_vals[1], i),
                                    broadcast_at(&input_vals[2], i),
                                )
                            })
                            .collect(),
                    ),
                }
            }
            (BlockKind::CoordinateTransform { kind, output_names }, _) => {
                let flat = flatten(&input_vals);
                let outs = kind.call(&flat);
                // Same convention as BlockKind::CScript below: the block's own name is bound to
                // the primary (first) output, any remaining output_names are inserted directly
                // under their own names. Output side stays scalar-only by design -- see
                // book/dev-guide/src/vector-signals.md, category 9.
                for (name, v) in output_names.iter().zip(outs.iter()).skip(1) {
                    outputs.insert(name.clone(), SignalValue::Scalar(*v));
                }
                SignalValue::Scalar(outs[0])
            }
            (BlockKind::Pid { clamp, .. }, BlockState::Dynamic { state_space, x }) => {
                let scalars = require_all_scalar(&block.name, &input_vals)?;
                let (lo, hi) = match clamp {
                    PidClamp::Fixed(lo, hi) => (*lo, *hi),
                    PidClamp::Dynamic => (scalars[1], scalars[2]),
                };
                let error = [scalars[0]];
                let tentative_x = state_space.rk4_step(x, &error, dt);
                let tentative_output = state_space.output(&tentative_x, &error)[0];
                let saturating_further = (tentative_output >= hi && error[0] > 0.0)
                    || (tentative_output <= lo && error[0] < 0.0);
                SignalValue::Scalar(if saturating_further {
                    state_space.output(x, &error)[0].clamp(lo, hi)
                } else {
                    *x = tentative_x;
                    tentative_output
                })
            }
            (BlockKind::TransferFunction(_), BlockState::Dynamic { state_space, x }) => {
                // Genuinely SISO by definition -- a rational N(s)/D(s) has no matrix
                // generalization the way StateSpace's own (A,B,C,D) does. Reject a Vector input
                // the same way Pid/Vco/Hysteresis do, rather than silently flattening.
                let u = require_all_scalar(&block.name, &input_vals)?;
                *x = state_space.rk4_step(x, &u, dt);
                SignalValue::Scalar(state_space.output(x, &u)[0])
            }
            (BlockKind::StateSpace(_), BlockState::Dynamic { state_space, x }) => {
                // Genuinely MIMO: state_space.inputs()/outputs() are not constrained to 1 --
                // see BlockKind::StateSpace's own doc comment. Inputs flatten the same way
                // CoordinateTransform/Pmsm/cscript's do; the *total* flattened length must
                // match this system's own declared input count exactly (a mismatch can only be
                // known here, not at parse time, since a vector-valued upstream signal's own
                // length isn't visible from netlist text alone).
                let u = flatten(&input_vals);
                let expected = state_space.inputs();
                if u.len() != expected {
                    return Err(DaeError::VectorSignalSizeMismatch {
                        block: block.name.clone(),
                        expected,
                        got: u.len(),
                    });
                }
                *x = state_space.rk4_step(x, &u, dt);
                let y = state_space.output(x, &u);
                if y.len() == 1 {
                    SignalValue::Scalar(y[0])
                } else {
                    SignalValue::Vector(y)
                }
            }
            (BlockKind::Vco(_), BlockState::Vco { vco, phase }) => {
                let u = require_all_scalar(&block.name, &input_vals)?;
                *phase = vco.step(*phase, u[0], dt);
                SignalValue::Scalar(*phase)
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
                let u = require_all_scalar(&block.name, &input_vals)?;
                let freq_command = u[0];
                let phase_offset = u[1];
                let duty = u[2];
                *phase = osc.step(*phase, freq_command, dt);
                let actual_freq = freq_command.clamp(osc.f_min, osc.f_max);
                let theta = (*phase + phase_offset).rem_euclid(1.0);
                let (main, complement) = math_ops::complementary_pwm_with_deadtime(
                    theta,
                    duty,
                    red * actual_freq,
                    fed * actual_freq,
                );
                SignalValue::Scalar(emit_complementary_pair(
                    &mut outputs,
                    output_names,
                    main,
                    complement,
                ))
            }
            (BlockKind::Pmsm { output_names, .. }, BlockState::Pmsm { pmsm, x }) => {
                let flat = flatten(&input_vals);
                *x = pmsm.step(*x, flat[0], flat[1], flat[2], dt);
                let [id, iq, omega_m, theta_e] = *x;
                let full = [id, iq, omega_m, Pmsm::theta_e_wrapped(theta_e)];
                for (name, v) in output_names.iter().zip(full.iter()).skip(1) {
                    outputs.insert(name.clone(), SignalValue::Scalar(*v));
                }
                SignalValue::Scalar(full[0])
            }
            (BlockKind::Hysteresis(_), BlockState::Hysteresis { hysteresis, on }) => {
                let u = require_all_scalar(&block.name, &input_vals)?;
                *on = hysteresis.step(*on, u[0]);
                SignalValue::Scalar(if *on { 1.0 } else { 0.0 })
            }
            (
                BlockKind::CScript {
                    output_names,
                    sample_time,
                    xc_count,
                    ..
                },
                BlockState::CScript {
                    instance,
                    time_since_sample,
                    last_output,
                    xc,
                },
            ) => {
                // sample_time = None: run every step, exactly like every other dynamic block.
                // sample_time = Some(ts): accumulate circuit dt until ts is reached, then run
                // once with the *accumulated* elapsed time as this call's dt (not the much
                // finer circuit dt), and hold the result (zero-order hold) on every step in
                // between -- see BlockKind::CScript's own doc comment for why. This same "due"
                // gating governs xc's own RK4 step too: a block that wants xc to integrate
                // continuously every circuit step should simply leave sample_time unset, the
                // same way any other continuously-evaluated block already works here -- xc
                // never advances on a step this block isn't "due" on, exactly like last_output
                // is held rather than recomputed on those steps.
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
                    // Flattened the same way StateSpace/CoordinateTransform/Pmsm's inputs are --
                    // each declared `inputs=` entry independently scalar or vector, concatenated
                    // in order into the flat array cscript_output/cscript_output_xc/
                    // cscript_derivative already expect. No C ABI change needed for this.
                    let flat = flatten(&input_vals);
                    *last_output = if *xc_count > 0 {
                        // Step xc first, then compute output from the *new* xc -- the same
                        // "step state, then compute output from the updated state" ordering
                        // BlockKind::StateSpace/TransferFunction's own arm above uses.
                        *xc = instance.rk4_step_xc(xc, &flat, elapsed);
                        instance.call_xc(&flat, elapsed, xc, output_names.len())
                    } else {
                        instance.call(&flat, elapsed, output_names.len())
                    };
                    // Optional, always a no-op if cscript_update wasn't exported -- the
                    // dedicated place `state`'s own discrete bookkeeping commits forward to the
                    // next step, called with this same step's already-integrated xc (empty when
                    // xc_count == 0). See cscript_ffi's own module doc comment, "The optional
                    // discrete-state update function."
                    instance.update(&flat, elapsed, xc);
                }
                // The primary value (this block's own name) is last_output[0], inserted below
                // like every other block; any additional declared output_names are inserted
                // here under their own names, so a downstream block can reference them
                // directly via Signal::Block(name) without needing to know they came from a
                // CScript block. Output side stays scalar-only by design (same as
                // CoordinateTransform/Pmsm above) -- see book/dev-guide/src/vector-signals.md,
                // category 10.
                for (name, v) in output_names.iter().zip(last_output.iter()).skip(1) {
                    outputs.insert(name.clone(), SignalValue::Scalar(*v));
                }
                SignalValue::Scalar(last_output.first().copied().unwrap_or(0.0))
            }
            (
                BlockKind::PyBlock {
                    output_names,
                    sample_time,
                    xc_count,
                    ..
                },
                BlockState::PyBlock {
                    instance,
                    time_since_sample,
                    last_output,
                    xc,
                },
            ) => {
                // Identical sample_time/xc gating to BlockKind::CScript's own arm above -- see
                // its own comment for the full rationale, unchanged here.
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
                    // Unlike CScript's own flat C array, each declared input keeps its own
                    // scalar/vector shape here -- Python can express "a list of values, each
                    // either a float or an ndarray" naturally, so there's no need to flatten
                    // (see pyblock_ffi's own module doc comment).
                    let py_inputs: Vec<pyblock_ffi::PyInput> = input_vals
                        .iter()
                        .map(|v| match v {
                            SignalValue::Scalar(x) => pyblock_ffi::PyInput::Scalar(*x),
                            SignalValue::Vector(xs) => pyblock_ffi::PyInput::Vector(xs),
                        })
                        .collect();
                    *last_output = if *xc_count > 0 {
                        *xc = instance
                            .rk4_step_xc(xc, t, &py_inputs, elapsed)
                            .map_err(DaeError::PyBlock)?;
                        instance
                            .call_xc(t, elapsed, &py_inputs, xc, output_names.len())
                            .map_err(DaeError::PyBlock)?
                    } else {
                        instance
                            .call(t, elapsed, &py_inputs, output_names.len())
                            .map_err(DaeError::PyBlock)?
                    };
                    // Optional, always a no-op if update() wasn't defined -- see cscript's own
                    // arm above and pyblock_ffi's module doc comment, "The optional
                    // discrete-state update function."
                    if *xc_count > 0 {
                        instance
                            .update_xc(t, elapsed, &py_inputs, xc)
                            .map_err(DaeError::PyBlock)?;
                    } else {
                        instance
                            .update(t, elapsed, &py_inputs)
                            .map_err(DaeError::PyBlock)?;
                    }
                }
                for (name, v) in output_names.iter().zip(last_output.iter()).skip(1) {
                    outputs.insert(name.clone(), SignalValue::Scalar(*v));
                }
                SignalValue::Scalar(last_output.first().copied().unwrap_or(0.0))
            }
            (
                BlockKind::PyFunction {
                    output_names,
                    sample_time,
                    ..
                },
                BlockState::PyFunction {
                    instance,
                    time_since_sample,
                    last_output,
                },
            ) => {
                // Same sample_time gating as every other zero-order-hold block, just for *when*
                // this block is due -- unlike every other gated block, the elapsed time itself
                // is discarded: PyFunctionInstance::call has no dt parameter at all (a pure
                // function has no notion of elapsed time to hand it), so only `due` matters here.
                let due = match sample_time {
                    None => true,
                    Some(ts) => {
                        *time_since_sample += dt;
                        if *time_since_sample + 1e-15 >= *ts {
                            *time_since_sample = 0.0;
                            true
                        } else {
                            false
                        }
                    }
                };
                if due {
                    let py_inputs: Vec<pyblock_ffi::PyInput> = input_vals
                        .iter()
                        .map(|v| match v {
                            SignalValue::Scalar(x) => pyblock_ffi::PyInput::Scalar(*x),
                            SignalValue::Vector(xs) => pyblock_ffi::PyInput::Vector(xs),
                        })
                        .collect();
                    *last_output = instance
                        .call(&py_inputs, output_names.len())
                        .map_err(DaeError::PyBlock)?;
                }
                for (name, v) in output_names.iter().zip(last_output.iter()).skip(1) {
                    outputs.insert(name.clone(), SignalValue::Scalar(*v));
                }
                SignalValue::Scalar(last_output.first().copied().unwrap_or(0.0))
            }
            _ => unreachable!("BlockState variant always matches its BlockKind"),
        };
        outputs.insert(block.name.clone(), value);
    }
    Ok(outputs)
}

fn resolve_gates(
    gates: &BTreeMap<String, GateBinding>,
    outputs: &BTreeMap<String, SignalValue>,
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
    let statements = general_mna::parse_and_flatten(source, dialect).map_err(DaeError::Parse)?;

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
            // A GateBinding's target must be an explicit Sig2Voltage converter, never a raw
            // control block directly -- a MOSFET's gate is itself a voltage, so it shares the
            // same Signal-to-PS boundary a V-source's own magnitude uses (see
            // BlockKind::Sig2Voltage's own doc comment); no separate gate-only converter type
            // exists. This also correctly rejects naming a CScript/CoordinateTransform/Pmsm
            // block's own *extra* output alias directly (block_names maps those to the same
            // index, whose kind is never Sig2Voltage), so no separate check is needed for that
            // case.
            if !matches!(blocks[idx].kind, BlockKind::Sig2Voltage) {
                return Err(DaeError::GateTargetNotSig2Voltage {
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
        crate::build_with_mosfets(&statements, dialect, diodes, &initial_states, shared_r_on)?;

    // Signal-to-PS enforcement for a `V`/`I` source's own literal value: `general-mna` already
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
        let general_mna::Expression::Symbol(sym) = expr else {
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
    let mut pyblock_registry = pyblock_ffi::PyBlockRegistry::new();
    let mut pyfunction_registry = pyblock_ffi::PyFunctionRegistry::new();
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
                xc_count,
            } => {
                // xc_count == 0: the plain, single-function contract (cscript_output),
                // unchanged from before xc existed. xc_count > 0: the continuous-state
                // contract (cscript_derivative/cscript_output_xc instead) -- see
                // cscript_ffi::CScriptRegistry::{instantiate,instantiate_xc}'s own doc
                // comments for exactly which symbols each requires.
                let instance = if *xc_count > 0 {
                    cscript_registry.instantiate_xc(lib)
                } else {
                    cscript_registry.instantiate(lib)
                }
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
                    // Starts at rest, matching every other dynamic block's own convention
                    // (Pid/StateSpace/TransferFunction/Pmsm/Vco all start their own state at
                    // zero too).
                    xc: vec![0.0; *xc_count],
                }
            }
            BlockKind::PyBlock {
                path,
                output_names,
                sample_time,
                xc_count,
            } => {
                // Same xc_count == 0 / > 0 contract split as CScript's own arm above; no
                // "requires clone" upfront check here -- pyblock_ffi's own clone is generic
                // (Python's copy.deepcopy), no author opt-in needed, so it's never missing.
                let instance = if *xc_count > 0 {
                    pyblock_registry.instantiate_xc(path)
                } else {
                    pyblock_registry.instantiate(path)
                }
                .map_err(DaeError::PyBlock)?;
                BlockState::PyBlock {
                    instance,
                    time_since_sample: sample_time.unwrap_or(0.0),
                    last_output: vec![0.0; output_names.len()],
                    xc: vec![0.0; *xc_count],
                }
            }
            BlockKind::PyFunction {
                path,
                function,
                output_names,
                sample_time,
            } => {
                // Genuinely separate registry from pyblock's own -- see
                // BlockKind::PyFunction's own doc comment for why this isn't just PyBlock with
                // xc_count implicitly 0.
                let instance = pyfunction_registry
                    .instantiate(path, function)
                    .map_err(DaeError::PyBlock)?;
                BlockState::PyFunction {
                    instance,
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
    let mut prev_outputs: BTreeMap<String, SignalValue> = BTreeMap::new();

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
                    crate::build_with_mosfets(&statements, dialect, diodes, &states, shared_r_on)?;
                let (point, used_backward_euler) = step_with_fallback(
                    &system,
                    &statements,
                    &all_diodes,
                    x_prev_prev.as_deref(),
                    &x_prev,
                    dt,
                    &prev_diode_raw_ioff,
                    &prev_segments,
                    forced,
                    t,
                    &scalar_only(&outputs),
                    &scalar_only(&prev_outputs),
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

                    let (system, all_diodes) = crate::build_with_mosfets(
                        &statements,
                        dialect,
                        diodes,
                        &states,
                        shared_r_on,
                    )?;
                    let attempt = step_control::lte_attempt(
                        &system,
                        &statements,
                        &all_diodes,
                        x_prev_prev.as_deref(),
                        &x_prev,
                        dt,
                        &prev_diode_raw_ioff,
                        &prev_segments,
                        forced,
                        &config,
                        t,
                        &scalar_only(&outputs),
                        &scalar_only(&prev_outputs),
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
