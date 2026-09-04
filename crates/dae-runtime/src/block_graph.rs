//! Evaluates a graph of `general_mna::block_graph` types (`BlockKind`/`BlockInstance`/`Signal`/
//! `GateBinding` — `Const`, `Time`, `Pwc`, `Pwl`, `Sin`, `Pulse`, `Exp`, `Sffm`, `Sum`, `Gain`,
//! `Pid`, `StateSpace`, `TransferFunction`, `Vco`, `Pwm`, `PhaseShiftPwm`, `Product`,
//! `Saturation`, `Table`, `MathFn1`/`2`/`3`, `Hysteresis`, `CoordinateTransform`, `Pmsm`,
//! `CScript`, `Phys2Sig`, `Sig2Phys`), resolving every ideal switch's gate
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
//! [`BlockKind::Phys2Sig`] of the circuit's own state (the historically "closed-loop" case) or is
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
//! `general-simulator-cli` unconditionally for every ideal switch-containing transient run.

use std::collections::BTreeMap;

use continuous_blocks::logic::{as_bool, from_bool, rising_edge, srlatch_next};
use continuous_blocks::{math_ops, DiscretePidState, Hysteresis, Pmsm, StateSpace, Vco};
use cscript_ffi::CScriptRegistry;
use general_spice_core::Dialect;
use pwl_devices::{IdealDiode, IdealSwitch};

use crate::{
    classify_segments, sawtooth_carrier, step_control, step_with_fallback, DaeError, GateState,
    OperatingPoint, Segment, TimeStep, RINGING_COOLDOWN_STEPS,
};

use general_mna::block_graph::block_kind_name;
pub use general_mna::block_graph::{
    BlockInstance, BlockKind, ConstValue, GainValue, GateBinding, Phys2SigTarget, PhysicalDomain,
    PidClamp, SampleTimeSpec, Signal, SignalValue,
};

/// Hard ceiling on accepted [`TimeStep::Adaptive`] steps for one [`simulate_transient_with_blocks`]
/// call — see [`DaeError::AdaptiveStepStalled`]'s own doc comment for why this exists. Sized
/// generously above any legitimate run this project has actually needed (the largest real
/// fixed-step run on record is 1,000,000 rows for a 20ms/`dt=2e-8` transient; adaptive stepping
/// should need far fewer accepted steps than that to cover the same window) while staying well
/// under what would exhaust a typical machine's RAM before this check can fire (each row holds
/// every node voltage/branch current/block output as `f64`s — a few hundred bytes for a
/// medium-sized circuit — so even 10,000,000 rows stays in the hundreds-of-MB-to-low-GB range,
/// not the tens-of-GB range that actually crashed a machine running the unpatched bug this
/// guards against).
pub const ADAPTIVE_STEP_HARD_CAP: usize = 10_000_000;

/// Every input must be `Scalar` — the default rule for a `BlockKind` that has no elementwise/
/// broadcast/flattening rule of its own (`Pid`, `Vco`, `Hysteresis`, `TransferFunction`, `Pwm`,
/// `PhaseShiftPwm`, `Sig2Phys`). Returns the plain `f64` values in
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
/// that can legitimately appear there are a `Sig2Phys` converter's own name,
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
    /// Shared by [`BlockKind::DiscreteStateSpace`] and [`BlockKind::DiscreteTransferFunction`] —
    /// the discrete-domain counterpart to [`Self::Dynamic`] above: `x` advances via
    /// [`continuous_blocks::StateSpace::discrete_step`] instead of `rk4_step`, only on steps this
    /// block's own mandatory `sample_time` says it's due (see [`BlockKind::CScript`]'s own
    /// zero-order-hold gating in `evaluate_blocks`, reused unchanged here). `last_output` is what
    /// gets returned on every step this block *isn't* due — a genuinely discrete system's own
    /// output is only defined at its own sample instants, not recomputable from a held `x` and a
    /// fresher `u` in between, unlike the continuous case.
    DiscreteDynamic {
        state_space: continuous_blocks::StateSpace,
        x: Vec<f64>,
        time_since_sample: f64,
        last_output: Vec<f64>,
    },
    /// [`BlockKind::DiscretePid`]'s own state — [`continuous_blocks::DiscretePidState`] plus the
    /// same zero-order-hold `time_since_sample`/`last_output` bookkeeping as
    /// [`Self::DiscreteDynamic`] above (this block's mandatory `sample_time` is where
    /// [`continuous_blocks::DiscretePid::period`] itself came from at construction — see
    /// `general-mna`'s own `system_builder.rs`).
    DiscretePid {
        state: continuous_blocks::DiscretePidState,
        time_since_sample: f64,
        last_output: f64,
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
    /// [`BlockKind::SrLatch`]'s own state -- a single persisted bit, updated every step
    /// (level-triggered, no clock/edge detection at all, unlike [`Self::FlipFlop`]/
    /// [`Self::Counter`] below).
    SrLatch {
        q: bool,
    },
    /// [`BlockKind::FlipFlop`]'s own state -- `q` is the flip-flop's own output, held unchanged
    /// except at a detected rising `clk` edge; `prev_clk` is this instance's own memory of the
    /// previous step's `clk` sample, the only way to detect that edge at all (see
    /// `logic-signals.md`'s own Category 3 for why this lives in `BlockState` rather than
    /// requiring a separate `prev:`-wired clock signal).
    FlipFlop {
        q: bool,
        prev_clk: f64,
    },
    /// [`BlockKind::Counter`]'s own state -- same rising-edge-detection skeleton as
    /// [`Self::FlipFlop`], generalized from a single bit to a signed running count.
    Counter {
        count: i64,
        prev_clk: f64,
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
        // Only meaningful for `SampleTimeSpec::Variable` -- the interval (seconds, relative)
        // `cscript_next_sample_hit` returned after this block's last due call, compared against
        // `time_since_sample` the same way `SampleTimeSpec::Periodic`'s own fixed `period` is.
        // Starts at `0.0` so a Variable block is unconditionally due on its first call, same
        // convention every other zero-order-hold block's own first call already has.
        next_hit_dt: f64,
        last_output: Vec<f64>,
        xc: Vec<f64>,
    },
    /// The Python-hosted counterpart to `CScript` above — same zero-order-hold `sample_time`
    /// bookkeeping, same `xc` continuous-state vector, `instance` a live
    /// [`pyblock_ffi::PyBlockInstance`] instead of a C one. Cloning this variant uses Python's
    /// own generic `copy.deepcopy` (see `pyblock_ffi`'s own module doc comment) — unlike
    /// `CScript`, there is no "doesn't support clone" panic path, since `deepcopy` needs no
    /// author opt-in. Only exists when the optional `python` feature is enabled — see
    /// `dae-runtime`'s own `Cargo.toml`.
    #[cfg(feature = "python")]
    PyBlock {
        instance: pyblock_ffi::PyBlockInstance,
        time_since_sample: f64,
        // Same role as `CScript`'s own `next_hit_dt` above.
        next_hit_dt: f64,
        last_output: Vec<f64>,
        xc: Vec<f64>,
    },
    /// The counterpart to `PyBlock` above for [`BlockKind::PyFunction`] — a genuinely separate
    /// variant (not reusing `PyBlock`'s own fields), since a stateless
    /// [`pyblock_ffi::PyFunctionInstance`] has no `xc` and cloning it is always trivially cheap
    /// (`clone_ref`, never `copy.deepcopy`, never fallible) — see that type's own doc comment.
    /// Still needs the same zero-order-hold `sample_time` bookkeeping every other zero-order-
    /// hold block has. Only exists when the optional `python` feature is enabled.
    #[cfg(feature = "python")]
    PyFunction {
        instance: pyblock_ffi::PyFunctionInstance,
        time_since_sample: f64,
        last_output: Vec<f64>,
    },
    /// The `.m`-file counterpart to `PyFunction` above, for [`BlockKind::OctFunc`] — same
    /// stateless contract, but `session` is an `Rc<RefCell<_>>` around one shared
    /// [`octave_ffi::OctaveSession`], not an owned interpreter handle: unlike `pyblock_ffi`'s
    /// embedded Python (process-global via PyO3, so every `PyFunctionInstance` reaches the same
    /// interpreter automatically), `octave_ffi::OctaveSession` is a real, separate `octave-cli`
    /// child process this crate itself owns and must explicitly share across every `OctFunc`
    /// block instance in the same run (see [`simulate_transient_with_blocks`]'s own construction
    /// loop, which spawns exactly one `OctaveSession`, lazily, the first time an `OctFunc` block
    /// is actually encountered, and hands every instance an `Rc::clone` of it). Cloning this
    /// variant (needed only for [`TimeStep::Adaptive`]'s retry loop) is always cheap and
    /// infallible — an `Rc::clone`, sharing the same underlying process and its own
    /// per-process-lifetime call-id counter, never spawning a second `octave-cli`; this is
    /// correct because [`octave_ffi::OctaveSession::call`] carries no state a rejected trial
    /// step could corrupt (see the module doc comment, "The call protocol" — nothing is ever
    /// assigned into the shared Octave workspace).
    OctFunction {
        session: std::rc::Rc<std::cell::RefCell<octave_ffi::OctaveSession>>,
        time_since_sample: f64,
        last_output: Vec<f64>,
    },
    /// The stateful counterpart to `OctFunction` above, for [`BlockKind::OctBlock`] -- same
    /// shared, `Rc`-cloned session every other Octave-hosted block kind in this run uses (see
    /// `OctFunction`'s own doc comment for why sharing is safe/correct even under
    /// [`TimeStep::Adaptive`]'s clone-per-trial-step loop), but unlike `OctFunction`, this
    /// block's own opaque per-instance state genuinely lives *inside* that shared session (one
    /// global struct, keyed by `instance` -- see `octave_ffi::OctaveSession::call_start`'s own
    /// doc comment), never inside this Rust struct at all. `instance` is this block's own
    /// `.name` -- the key into that shared struct -- and `xc` is the block's own solver-owned
    /// continuous-state vector, exactly the same role `CScript`'s/`PyBlock`'s own `xc` field
    /// has (empty when `xc_count == 0`). `time_since_sample`/`next_hit_dt`/`last_output` are the
    /// same zero-order-hold bookkeeping every other sample-time-gated block variant already
    /// carries.
    OctBlock {
        session: std::rc::Rc<std::cell::RefCell<octave_ffi::OctaveSession>>,
        instance: String,
        time_since_sample: f64,
        next_hit_dt: f64,
        last_output: Vec<f64>,
        xc: Vec<f64>,
        /// Whether `<function>_update.m` exists in this block's own `path` directory -- checked
        /// once, up front, at construction (see [`simulate_transient_with_blocks`]'s own
        /// construction loop), so `evaluate_blocks` never has to speculatively call an
        /// `_update.m` that might not exist (which would otherwise surface as a spurious
        /// Octave-side "undefined function" error for the common case where a block simply has
        /// no discrete bookkeeping to commit).
        has_update: bool,
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
            | BlockKind::OctFunc { output_names, .. }
            | BlockKind::OctBlock { output_names, .. }
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
/// `BlockKind::Phys2Sig`/`Signal::BlockPrev` inputs never contribute a dependency edge here —
/// `Phys2Sig` has zero inputs (a source block, like `Const`/`Time`) and `BlockPrev` reads state
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
/// in place at step size `dt`, reading any [`BlockKind::Phys2Sig`] from `point_prev` and any
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

/// The zero-order-hold "due" threshold shared by [`BlockKind::CScript`]/[`BlockKind::PyBlock`]/
/// [`BlockKind::OctBlock`]'s own per-step gating: `None` sample_time runs continuously (no
/// threshold at all), `Periodic` uses its fixed `period`, `Variable` uses whatever `next_hit_dt`
/// this instance's own `next_sample_hit` last reported. Factored out so
/// [`earliest_next_event_dt`] can reuse the exact same rule its callers' own "due" checks use,
/// rather than re-deriving it.
fn sample_threshold(sample_time: Option<&SampleTimeSpec>, next_hit_dt: f64) -> Option<f64> {
    match sample_time {
        None => None,
        Some(SampleTimeSpec::Periodic { period, .. }) => Some(*period),
        Some(SampleTimeSpec::Variable) => Some(next_hit_dt),
    }
}

/// Smallest non-negative relative `dt` (from "now") at which a zero-order-hold block next
/// becomes due, given its currently-persisted `time_since_sample` accumulator and (already
/// resolved) `threshold`. `None` propagates through (never due on its own -- e.g. `sample_time
/// == None`, run-every-step blocks aren't scheduled events for this purpose since they have no
/// discrete "next hit" at all).
fn time_until_due(threshold: Option<f64>, time_since_sample: f64) -> Option<f64> {
    threshold.map(|th| (th - time_since_sample).max(0.0))
}

/// The smallest relative `dt` (from theta_now, in the same `[0,1)`-wrapped phase space
/// `sawtooth_carrier`/`Vco::step` use) at which one of `duty`/`Pwm`'s complementary-pair
/// dead-time thresholds is next crossed, converted to seconds via `freq` (Hz). Shared by
/// [`BlockKind::Pwm`] and [`BlockKind::PhaseShiftPwm`]'s own [`earliest_next_event_dt`] arms --
/// both use the identical `{red_frac, duty, duty + fed_frac}` threshold set
/// [`continuous_blocks::math_ops::complementary_pwm_with_deadtime`] itself compares against.
/// `None` if `freq` isn't usable (non-positive or non-finite -- would make `delta_t` meaningless
/// or divide by zero).
///
/// A threshold this function itself just reported (and the adaptive loop then landed a step
/// exactly on, which is the whole point of calling this at all) reads back as `theta_now ==
/// that threshold` on the very next call -- naively that's `delta_theta == 0`, i.e. "you're
/// already there," which the adaptive loop's own `ev.max(1e-15)` clamp turns into a real ~1fs
/// step instead of an error. Theta barely moves in a step that small, so the SAME threshold
/// reads back as ~0 again next iteration too: an unbounded near-zero-dt loop that silently
/// consumes memory (one row per iteration, unbounded) until the process is killed -- this
/// crashed the machine it ran on once already. `THETA_EPS` (a full period's worth of
/// `f64::EPSILON`-scale noise, times a wide safety margin -- see the constant's own comment)
/// shifts the search so a threshold within `THETA_EPS` *behind* `theta_now` reads as "already
/// crossed, wait a full period," never as "0 away" -- while any threshold more than `THETA_EPS`
/// ahead is unaffected to double-precision noise. Regression test:
/// `adaptive_event_landing::stepping_repeatedly_lands_exactly_on_the_same_edge_without_stalling`.
fn next_pwm_edge_dt(
    theta_now: f64,
    duty: f64,
    red_frac: f64,
    fed_frac: f64,
    freq: f64,
) -> Option<f64> {
    // theta = (t * freq_hz).fract() (see `sawtooth_carrier`) or a phase accumulator advanced by
    // `freq * dt` each step (see `Vco::step`) -- both accumulate error on the order of
    // `f64::EPSILON * theta_now.recip_scale()`, comfortably under 1e-9 for any `t`/`phase`
    // magnitude this engine's own `AdaptiveConfig::from_t_final` would ever produce (t_final up
    // to ~1e6s would still keep `t * freq_hz` under 1e18, whose `f64::EPSILON`-scale noise is
    // ~1e2 in absolute terms -- i.e. this bound would need revisiting only for a `t_final`/`freq`
    // combination many orders of magnitude past anything this project runs).
    const THETA_EPS: f64 = 1e-9;
    if !freq.is_finite() || freq <= 0.0 {
        return None;
    }
    let thresholds = [red_frac, duty, duty + fed_frac];
    let delta_theta = thresholds
        .iter()
        .map(|&r| (r - theta_now - THETA_EPS).rem_euclid(1.0) + THETA_EPS)
        .fold(f64::INFINITY, f64::min);
    if !delta_theta.is_finite() {
        return None;
    }
    Some(delta_theta / freq)
}

/// Resolves `signal` against `prev_outputs` (the previous *accepted* step's own output map, not
/// a trial-in-progress one) as a scalar, or `None` if unresolvable (unknown name, vector-valued,
/// or -- on the very first step, before any step has been accepted -- simply not there yet).
/// `Signal::Block` and `Signal::BlockPrev` are treated identically here: both name the same
/// "last known value" this function wants, unlike `evaluate_blocks`' own `resolve` closure,
/// which distinguishes "this step's already-computed upstream output" from "strictly the
/// previous step's" -- a distinction [`earliest_next_event_dt`] has no use for, since it never
/// computes a same-step `outputs` map of its own at all.
fn resolve_prev_scalar(
    prev_outputs: &BTreeMap<String, SignalValue>,
    signal: &Signal,
) -> Option<f64> {
    let name = match signal {
        Signal::Block(n) | Signal::BlockPrev(n) => n,
    };
    prev_outputs.get(name).and_then(|v| v.as_scalar())
}

/// Computes the earliest relative `dt` (from `t`, using each block's currently-*persisted*
/// state -- never a trial evaluation) at which some in-scope block's output would next cross a
/// discrete threshold (a `Pwm`/`PhaseShiftPwm` gate edge, or a zero-order-hold block's next
/// sample hit), assuming every block's own runtime inputs stay at their last-*evaluated* values
/// (i.e. `prev_outputs`, the previous accepted step's own outputs). Returns `None` if no in-scope
/// block reports a finite estimate.
///
/// Read fresh, from `block_states`/`prev_outputs` as they stand at the very start of every
/// accepted-step iteration of [`TimeStep::Adaptive`]'s own loop -- never extrapolated across
/// multiple steps -- so a stale prediction (an input that changes again before the predicted
/// edge, e.g. `PhaseShiftPwm`'s closed-loop-driven `freq_command`/`duty`) is bounded to at most
/// one step's own error, which is strictly better than today's zero prediction, never worse: this
/// is a *clamp* on the adaptive step, not a substitute for the LTE-driven retry loop that still
/// runs after it and can shrink `dt` further.
///
/// [`BlockKind::Vco`] is deliberately excluded (always contributes `None`) -- its own output is
/// the raw phase itself (a continuous ramp `[0,1)`, not a discrete/boolean gate signal), so the
/// only "discontinuity" it has is the phase-wrap seam, which is a slope/value jump in a ramp
/// signal feeding into some *other* block, not itself a gate transition a downstream comparator
/// could silently swallow the way a missed `Pwm`/`PhaseShiftPwm` edge is. Out of scope for this
/// feature.
///
/// `kind=octblock` stays out of scope for `TimeStep::Adaptive` entirely (see
/// [`simulate_transient_with_blocks`]'s own upfront rejection) -- this function still computes an
/// estimate for it (reusing the same `sample_threshold`/`time_until_due` helpers CScript/PyBlock
/// use) for whenever that separate restriction is eventually lifted, but it is unreachable in
/// practice today.
fn earliest_next_event_dt(
    blocks: &[BlockInstance],
    block_states: &[BlockState],
    prev_outputs: &BTreeMap<String, SignalValue>,
    t: f64,
) -> Option<f64> {
    blocks
        .iter()
        .zip(block_states.iter())
        .filter_map(|(block, state)| match (&block.kind, state) {
            (
                BlockKind::Pwm {
                    freq_hz, red, fed, ..
                },
                _,
            ) => {
                let theta_now = sawtooth_carrier(t, *freq_hz);
                let duty = resolve_prev_scalar(prev_outputs, block.inputs.first()?)?;
                next_pwm_edge_dt(theta_now, duty, red * freq_hz, fed * freq_hz, *freq_hz)
            }
            (
                BlockKind::PhaseShiftPwm { red, fed, .. },
                BlockState::PhaseShiftPwm { osc, phase },
            ) => {
                let freq_command = resolve_prev_scalar(prev_outputs, block.inputs.first()?)?;
                let phase_offset = resolve_prev_scalar(prev_outputs, block.inputs.get(1)?)?;
                let duty = resolve_prev_scalar(prev_outputs, block.inputs.get(2)?)?;
                let actual_freq = freq_command.clamp(osc.f_min, osc.f_max);
                let theta_now = (*phase + phase_offset).rem_euclid(1.0);
                next_pwm_edge_dt(
                    theta_now,
                    duty,
                    red * actual_freq,
                    fed * actual_freq,
                    actual_freq,
                )
            }
            // BlockKind::Vco: deliberately excluded, see this function's own doc comment.
            (
                BlockKind::CScript { sample_time, .. },
                BlockState::CScript {
                    time_since_sample,
                    next_hit_dt,
                    ..
                },
            ) => time_until_due(
                sample_threshold(sample_time.as_ref(), *next_hit_dt),
                *time_since_sample,
            ),
            #[cfg(feature = "python")]
            (
                BlockKind::PyBlock { sample_time, .. },
                BlockState::PyBlock {
                    time_since_sample,
                    next_hit_dt,
                    ..
                },
            ) => time_until_due(
                sample_threshold(sample_time.as_ref(), *next_hit_dt),
                *time_since_sample,
            ),
            #[cfg(feature = "python")]
            (
                BlockKind::PyFunction { sample_time, .. },
                BlockState::PyFunction {
                    time_since_sample, ..
                },
            ) => match sample_time {
                None | Some(SampleTimeSpec::Variable) => None,
                Some(SampleTimeSpec::Periodic { period, .. }) => {
                    Some((period - time_since_sample).max(0.0))
                }
            },
            (
                BlockKind::OctFunc { sample_time, .. },
                BlockState::OctFunction {
                    time_since_sample, ..
                },
            ) => match sample_time {
                None | Some(SampleTimeSpec::Variable) => None,
                Some(SampleTimeSpec::Periodic { period, .. }) => {
                    Some((period - time_since_sample).max(0.0))
                }
            },
            (
                BlockKind::OctBlock { sample_time, .. },
                BlockState::OctBlock {
                    time_since_sample,
                    next_hit_dt,
                    ..
                },
            ) => time_until_due(
                sample_threshold(sample_time.as_ref(), *next_hit_dt),
                *time_since_sample,
            ),
            (
                BlockKind::DiscreteStateSpace { sample_time, .. }
                | BlockKind::DiscreteTransferFunction { sample_time, .. },
                BlockState::DiscreteDynamic {
                    time_since_sample, ..
                },
            ) => match sample_time {
                SampleTimeSpec::Periodic { period, .. } => {
                    Some((period - time_since_sample).max(0.0))
                }
                SampleTimeSpec::Variable => None, // rejected at parse time for this kind
            },
            (
                BlockKind::DiscretePid { sample_time, .. },
                BlockState::DiscretePid {
                    time_since_sample, ..
                },
            ) => match sample_time {
                SampleTimeSpec::Periodic { period, .. } => {
                    Some((period - time_since_sample).max(0.0))
                }
                SampleTimeSpec::Variable => None, // rejected at parse time for this kind
            },
            _ => None,
        })
        .fold(None, |acc, dt| match acc {
            None => Some(dt),
            Some(best) => Some(best.min(dt)),
        })
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
            (BlockKind::Phys2Sig(target), _) => SignalValue::Scalar(match target {
                Phys2SigTarget::Voltage(node) => {
                    point_prev.value(&format!("V({node})")).unwrap_or(0.0)
                }
                Phys2SigTarget::Current(branch) => {
                    point_prev.value(&format!("I({branch})")).unwrap_or(0.0)
                }
            }),
            (BlockKind::Sig2Phys { .. }, _) => {
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
            (
                BlockKind::DiscreteStateSpace { sample_time, .. },
                BlockState::DiscreteDynamic {
                    state_space,
                    x,
                    time_since_sample,
                    last_output,
                },
            ) => {
                // Genuinely MIMO, same flattening rule as BlockKind::StateSpace's own continuous
                // arm above -- but a discrete system's own output is only defined at its own
                // sample instants (see BlockKind::CScript's zero-order-hold gating, reused
                // unchanged here): `last_output` is held on every step this block isn't due.
                let period = match sample_time {
                    SampleTimeSpec::Periodic { period, .. } => *period,
                    SampleTimeSpec::Variable => unreachable!(
                        "general-mna's parse_required_periodic_sample_time already rejected \
                         ts=variable for kind=discretestatespace"
                    ),
                };
                *time_since_sample += dt;
                if *time_since_sample + 1e-15 >= period {
                    *time_since_sample = 0.0;
                    let u = flatten(&input_vals);
                    let expected = state_space.inputs();
                    if u.len() != expected {
                        return Err(DaeError::VectorSignalSizeMismatch {
                            block: block.name.clone(),
                            expected,
                            got: u.len(),
                        });
                    }
                    *x = state_space.discrete_step(x, &u);
                    *last_output = state_space.output(x, &u);
                }
                if last_output.len() == 1 {
                    SignalValue::Scalar(last_output[0])
                } else {
                    SignalValue::Vector(last_output.clone())
                }
            }
            (
                BlockKind::DiscreteTransferFunction { sample_time, .. },
                BlockState::DiscreteDynamic {
                    state_space,
                    x,
                    time_since_sample,
                    last_output,
                },
            ) => {
                // Genuinely SISO, same rejection of a Vector input as the continuous
                // BlockKind::TransferFunction arm above.
                let period = match sample_time {
                    SampleTimeSpec::Periodic { period, .. } => *period,
                    SampleTimeSpec::Variable => unreachable!(
                        "general-mna's parse_required_periodic_sample_time already rejected \
                         ts=variable for kind=discretetf"
                    ),
                };
                *time_since_sample += dt;
                if *time_since_sample + 1e-15 >= period {
                    *time_since_sample = 0.0;
                    let u = require_all_scalar(&block.name, &input_vals)?;
                    *x = state_space.discrete_step(x, &u);
                    *last_output = state_space.output(x, &u);
                }
                SignalValue::Scalar(last_output.first().copied().unwrap_or(0.0))
            }
            (
                BlockKind::DiscretePid {
                    pid,
                    clamp,
                    sample_time,
                },
                BlockState::DiscretePid {
                    state,
                    time_since_sample,
                    last_output,
                },
            ) => {
                let period = match sample_time {
                    SampleTimeSpec::Periodic { period, .. } => *period,
                    SampleTimeSpec::Variable => unreachable!(
                        "general-mna's parse_required_periodic_sample_time already rejected \
                         ts=variable for kind=discretepid"
                    ),
                };
                let scalars = require_all_scalar(&block.name, &input_vals)?;
                let (lo, hi) = match clamp {
                    PidClamp::Fixed(lo, hi) => (*lo, *hi),
                    PidClamp::Dynamic => (scalars[1], scalars[2]),
                };
                let error = scalars[0];
                *time_since_sample += dt;
                if *time_since_sample + 1e-15 >= period {
                    *time_since_sample = 0.0;
                    // Same tentative-step/reject anti-windup as BlockKind::Pid's own continuous
                    // arm above, adapted to a discrete recursion: step a *copy* of the state,
                    // and only commit it if doing so wouldn't push the output further past
                    // whichever bound it's already saturating against -- the discrete
                    // counterpart to "don't integrate the error further while already clamped."
                    let mut tentative_state = *state;
                    let tentative_output = pid.step(&mut tentative_state, error);
                    let saturating_further = (tentative_output >= hi && error > 0.0)
                        || (tentative_output <= lo && error < 0.0);
                    *last_output = if saturating_further {
                        tentative_output.clamp(lo, hi)
                    } else {
                        *state = tentative_state;
                        tentative_output
                    };
                }
                SignalValue::Scalar(*last_output)
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
            (BlockKind::LogicGate(op), _) => {
                // Purely combinational -- no BlockState variant at all (matches `_` above,
                // same as Sum/Gain), recomputed fresh from this step's own inputs every time.
                let u = require_all_scalar(&block.name, &input_vals)?;
                let bits: Vec<bool> = u.iter().map(|&x| as_bool(x)).collect();
                SignalValue::Scalar(from_bool(op.call(&bits)))
            }
            (BlockKind::SrLatch { priority }, BlockState::SrLatch { q }) => {
                let u = require_all_scalar(&block.name, &input_vals)?;
                let (set, reset) = (as_bool(u[0]), as_bool(u[1]));
                *q = srlatch_next(*q, set, reset, *priority);
                SignalValue::Scalar(from_bool(*q))
            }
            (BlockKind::FlipFlop { kind, .. }, BlockState::FlipFlop { q, prev_clk }) => {
                let u = require_all_scalar(&block.name, &input_vals)?;
                let clk = u[0];
                // Input order, fixed by system_builder.rs's own parsing: [clk, data_inputs...,
                // reset?] -- `kind.input_count()` data inputs right after clk, an optional
                // trailing reset beyond that (present iff `u.len()` has one more entry than
                // `1 + kind.input_count()` accounts for).
                let data = &u[1..1 + kind.input_count()];
                let reset_asserted = u.get(1 + kind.input_count()).is_some_and(|&r| as_bool(r));
                if rising_edge(clk, *prev_clk) {
                    let bits: Vec<bool> = data.iter().map(|&x| as_bool(x)).collect();
                    *q = if reset_asserted {
                        false
                    } else {
                        kind.next_state(*q, &bits)
                    };
                }
                *prev_clk = clk;
                SignalValue::Scalar(from_bool(*q))
            }
            (
                BlockKind::Counter {
                    up_down, modulus, ..
                },
                BlockState::Counter { count, prev_clk },
            ) => {
                let u = require_all_scalar(&block.name, &input_vals)?;
                let clk = u[0];
                // Same fixed input order system_builder.rs's own "counter" arm builds:
                // [clk, up_down?, reset?] -- both optional, in that order when both present.
                let mut idx = 1;
                let up_down_asserted = if *up_down {
                    let v = as_bool(u[idx]);
                    idx += 1;
                    v
                } else {
                    true // no up_down= wired: always increments
                };
                let reset_asserted = u.get(idx).is_some_and(|&r| as_bool(r));
                if rising_edge(clk, *prev_clk) {
                    if reset_asserted {
                        *count = 0;
                    } else if up_down_asserted {
                        *count += 1;
                    } else {
                        *count -= 1;
                    }
                    if let Some(m) = modulus {
                        *count = count.rem_euclid(*m as i64);
                    }
                }
                *prev_clk = clk;
                SignalValue::Scalar(*count as f64)
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
                    next_hit_dt,
                    last_output,
                    xc,
                },
            ) => {
                // sample_time = None: run every step, exactly like every other dynamic block.
                // sample_time = Some(Periodic{period,offset}): accumulate circuit dt until the
                // current threshold is reached, then run once with the *accumulated* elapsed
                // time as this call's dt (not the much finer circuit dt), and hold the result
                // (zero-order hold) on every step in between -- see BlockKind::CScript's own
                // doc comment for why. sample_time = Some(Variable): identical accumulator
                // mechanics, but the threshold itself (`next_hit_dt`) is whatever the block's
                // own cscript_next_sample_hit last returned, instead of a fixed `period`. This
                // same "due" gating governs xc's own RK4 step too: a block that wants xc to
                // integrate continuously every circuit step should simply leave sample_time
                // unset, the same way any other continuously-evaluated block already works here
                // -- xc never advances on a step this block isn't "due" on, exactly like
                // last_output is held rather than recomputed on those steps.
                let threshold = sample_threshold(sample_time.as_ref(), *next_hit_dt);
                let (due, elapsed) = match threshold {
                    None => (true, dt),
                    Some(threshold) => {
                        *time_since_sample += dt;
                        if *time_since_sample + 1e-15 >= threshold {
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
                    if matches!(sample_time, Some(SampleTimeSpec::Variable)) {
                        *next_hit_dt = instance.next_sample_hit(&flat, xc);
                    }
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
            #[cfg(feature = "python")]
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
                    next_hit_dt,
                    last_output,
                    xc,
                },
            ) => {
                // Identical sample_time/xc gating to BlockKind::CScript's own arm above -- see
                // its own comment for the full rationale, unchanged here.
                let threshold = sample_threshold(sample_time.as_ref(), *next_hit_dt);
                let (due, elapsed) = match threshold {
                    None => (true, dt),
                    Some(threshold) => {
                        *time_since_sample += dt;
                        if *time_since_sample + 1e-15 >= threshold {
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
                    if matches!(sample_time, Some(SampleTimeSpec::Variable)) {
                        *next_hit_dt = if *xc_count > 0 {
                            instance
                                .next_sample_hit_xc(t, elapsed, &py_inputs, xc)
                                .map_err(DaeError::PyBlock)?
                        } else {
                            instance
                                .next_sample_hit(t, elapsed, &py_inputs)
                                .map_err(DaeError::PyBlock)?
                        };
                    }
                }
                for (name, v) in output_names.iter().zip(last_output.iter()).skip(1) {
                    outputs.insert(name.clone(), SignalValue::Scalar(*v));
                }
                SignalValue::Scalar(last_output.first().copied().unwrap_or(0.0))
            }
            #[cfg(feature = "python")]
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
                    // Variable is unreachable here -- rejected at parse time for PyFunction.
                    Some(SampleTimeSpec::Variable) => true,
                    Some(SampleTimeSpec::Periodic { period, .. }) => {
                        *time_since_sample += dt;
                        if *time_since_sample + 1e-15 >= *period {
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
            (
                BlockKind::OctFunc {
                    output_names,
                    sample_time,
                    function,
                    ..
                },
                BlockState::OctFunction {
                    session,
                    time_since_sample,
                    last_output,
                    ..
                },
            ) => {
                // Identical zero-order-hold "due" gating to PyFunction's own arm above -- see
                // its own comment for the full rationale. Same discard-elapsed-time rule too:
                // OctaveSession::call has no dt parameter, a pure function has no notion of
                // elapsed time to hand it.
                let due = match sample_time {
                    None => true,
                    // Variable is unreachable here -- rejected at parse time for OctFunc.
                    Some(SampleTimeSpec::Variable) => true,
                    Some(SampleTimeSpec::Periodic { period, .. }) => {
                        *time_since_sample += dt;
                        if *time_since_sample + 1e-15 >= *period {
                            *time_since_sample = 0.0;
                            true
                        } else {
                            false
                        }
                    }
                };
                if due {
                    // Unlike PyFunction (which keeps each declared input's own scalar/vector
                    // shape, via numpy), octave_ffi::OctaveSession::call takes a flat &[f64] of
                    // literal scalar arguments -- there is no vector-input support for octfunc
                    // (matching this crate's own scalar-only-outputs restriction, and the
                    // literal-numeric-argument protocol octave_ffi's own module doc comment
                    // documents), so every declared input must itself be a Scalar.
                    let flat_inputs = require_all_scalar(&block.name, &input_vals)?;
                    *last_output = session
                        .borrow_mut()
                        .call(function, &flat_inputs, output_names.len())
                        .map_err(DaeError::Octave)?;
                }
                for (name, v) in output_names.iter().zip(last_output.iter()).skip(1) {
                    outputs.insert(name.clone(), SignalValue::Scalar(*v));
                }
                SignalValue::Scalar(last_output.first().copied().unwrap_or(0.0))
            }
            (
                BlockKind::OctBlock {
                    output_names,
                    sample_time,
                    xc_count,
                    function,
                    ..
                },
                BlockState::OctBlock {
                    session,
                    instance,
                    time_since_sample,
                    next_hit_dt,
                    last_output,
                    xc,
                    has_update,
                },
            ) => {
                // Identical sample_time/xc "due" gating to CScript's/PyBlock's own arms above --
                // unlike OctFunc/PyFunction (stateless, elapsed time discarded), this block's
                // own contract functions take `t`/`dt` explicitly (see BlockKind::OctBlock's own
                // file-per-function table), so the accumulated `elapsed` matters here.
                let threshold = sample_threshold(sample_time.as_ref(), *next_hit_dt);
                let (due, elapsed) = match threshold {
                    None => (true, dt),
                    Some(threshold) => {
                        *time_since_sample += dt;
                        if *time_since_sample + 1e-15 >= threshold {
                            let elapsed = *time_since_sample;
                            *time_since_sample = 0.0;
                            (true, elapsed)
                        } else {
                            (false, 0.0)
                        }
                    }
                };
                if due {
                    // No vector-input support -- same scalar-only restriction OctFunc already
                    // has (see octave_ffi's own literal-numeric-argument protocol).
                    let flat_inputs = require_all_scalar(&block.name, &input_vals)?;
                    // `args` is [t, dt, u0, u1, ...] -- the exact positional shape every
                    // t/dt-taking contract function expects after its own leading `state`.
                    let mut args = Vec::with_capacity(2 + flat_inputs.len());
                    args.push(t);
                    args.push(elapsed);
                    args.extend_from_slice(&flat_inputs);

                    *last_output = if *xc_count > 0 {
                        // Step xc first (RK4, via <function>_derivative -- pure, never touches
                        // this instance's own opaque state slot), then compute output from the
                        // *new* xc -- same "step state, then compute output from the updated
                        // state" ordering CScript's/PyBlock's own xc arms use. Derivative's own
                        // args exclude t/dt (see the file-per-function table).
                        *xc = session
                            .borrow_mut()
                            .rk4_step_xc_stateful(
                                &format!("{function}_derivative"),
                                instance,
                                xc,
                                &flat_inputs,
                                elapsed,
                            )
                            .map_err(DaeError::Octave)?;
                        session
                            .borrow_mut()
                            .call_stateful(
                                &format!("{function}_output_xc"),
                                instance,
                                &args,
                                Some(xc),
                                output_names.len(),
                            )
                            .map_err(DaeError::Octave)?
                    } else {
                        session
                            .borrow_mut()
                            .call_stateful(function, instance, &args, None, output_names.len())
                            .map_err(DaeError::Octave)?
                    };
                    // Optional, a no-op if <function>_update.m wasn't found at construction --
                    // the dedicated place discrete bookkeeping commits forward to the next step,
                    // same role CScript's/PyBlock's own optional update has.
                    if *has_update {
                        session
                            .borrow_mut()
                            .call_stateful(&format!("{function}_update"), instance, &args, None, 0)
                            .map_err(DaeError::Octave)?;
                    }
                    if matches!(sample_time, Some(SampleTimeSpec::Variable)) {
                        *next_hit_dt = session
                            .borrow_mut()
                            .call_readonly_stateful(
                                &format!("{function}_next_sample_hit"),
                                instance,
                                &args,
                                None,
                                1,
                            )
                            .map_err(DaeError::Octave)?[0];
                    }
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

/// Rejects a `kind=sig2phys` converter that was *wired into* the physical network — i.e. a block
/// whose own name is also used as a node on some element line — before any system is built or any
/// step is solved. See [`DaeError::Sig2PhysUsedAsCircuitNode`] for the full rationale; the short
/// version is that a `Sig2Phys` block has no terminals and stamps nothing, so such a net is an
/// ordinary undriven node that silently solves to `0` instead of failing.
///
/// Call this with the *flattened* statements ([`general_mna::parse_and_flatten`]), so a converter
/// wired inside a `.subckt` body is caught under its final, flattened node name too.
///
/// Node names are compared case-insensitively (ASCII), because the netlist grammar itself treats
/// `A` and `a` as one node — writing `R1 vdrv 0 1k` against a converter declared `VDRV` is the
/// same mistake, and must produce the same error.
pub fn reject_sig2phys_wired_into_circuit(
    statements: &[general_spice_core::ast::Statement],
    blocks: &[BlockInstance],
) -> Result<(), DaeError> {
    // Only `Sig2Phys` blocks are checked, not every block kind: it is the only kind that claims
    // to drive the physical domain at all, so it is the only one anybody is tempted to draw a
    // wire from. (Wiring any *other* block's name as a node is the same silent-zero shape, but
    // rejecting every block/node name collision outright would also reject the harmless, and
    // plausible, habit of naming a `kind=phys2sig` probe after the node it measures.)
    let converters: BTreeMap<String, &str> = blocks
        .iter()
        .filter(|b| matches!(b.kind, BlockKind::Sig2Phys { .. }))
        .map(|b| (b.name.to_ascii_lowercase(), b.name.as_str()))
        .collect();
    if converters.is_empty() {
        return Ok(());
    }
    for statement in statements {
        let general_spice_core::ast::Statement::ElementInstance(element) = statement else {
            continue;
        };
        for node in &element.nodes {
            if let Some(declared) = converters.get(&node.to_ascii_lowercase()) {
                return Err(DaeError::Sig2PhysUsedAsCircuitNode {
                    block: (*declared).to_string(),
                    element: element.name.clone(),
                    node: node.clone(),
                });
            }
        }
    }
    Ok(())
}

fn resolve_gates(
    gates: &BTreeMap<String, GateBinding>,
    outputs: &BTreeMap<String, SignalValue>,
) -> BTreeMap<String, GateState> {
    gates
        .iter()
        .map(|(switch_name, binding)| (switch_name.clone(), binding.resolve(outputs)))
        .collect()
}

/// Runs a transient with every ideal switch's gate resolved from a [`GateBinding`] each step — see
/// this module's doc comment for why there's no separate "closed-loop" entry point: a device
/// gated by a plain [`BlockKind::Hysteresis`] and one gated by a `Sum`-`Pid`-[`BlockKind::PhaseShiftPwm`]
/// chain that happens to read a [`BlockKind::Phys2Sig`] are resolved by exactly the same loop
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
    diodes: &BTreeMap<String, IdealDiode>,
    ideal_switches: &BTreeMap<String, IdealSwitch>,
    blocks: &[BlockInstance],
    gates: &BTreeMap<String, GateBinding>,
    shared_r_on: f64,
    x_initial: Option<&[f64]>,
    t_final: f64,
    step: TimeStep,
) -> Result<Vec<TransientWithBlocksStep>, DaeError> {
    simulate_transient_with_blocks_capped(
        source,
        dialect,
        diodes,
        ideal_switches,
        blocks,
        gates,
        shared_r_on,
        x_initial,
        t_final,
        step,
        ADAPTIVE_STEP_HARD_CAP,
    )
}

/// Identical to [`simulate_transient_with_blocks`], with one addition: `max_adaptive_steps`
/// overrides [`ADAPTIVE_STEP_HARD_CAP`] (see [`DaeError::AdaptiveStepStalled`]'s own doc comment
/// for what this guards against) instead of always using the built-in default. A separate
/// function, rather than adding a parameter to `simulate_transient_with_blocks` itself, so this
/// project's own ~20 existing call sites (mostly tests, positional-argument calls) don't all need
/// updating for a cap most of them will never want to change — `general-simulator-cli`'s own
/// `--max-steps` flag is the intended caller.
///
/// Collects the whole run into a `Vec` before returning, exactly like `simulate_transient_with_blocks`
/// — a thin wrapper over [`simulate_transient_with_blocks_streamed`], which does the actual work
/// and holds nothing in memory beyond what its caller's own callback chooses to keep. Prefer the
/// streamed form directly for a run whose row count isn't known to be small (this repo's own
/// experience: a single real run drove a 14GB machine to ~11GB RSS / 24GB swap before being
/// killed, entirely because of this function's own full-`Vec` buffering — see
/// `general-simulator-cli`'s own streaming CSV/raw writer for the fix in practice).
#[allow(clippy::too_many_arguments)]
pub fn simulate_transient_with_blocks_capped(
    source: &str,
    dialect: Dialect,
    diodes: &BTreeMap<String, IdealDiode>,
    ideal_switches: &BTreeMap<String, IdealSwitch>,
    blocks: &[BlockInstance],
    gates: &BTreeMap<String, GateBinding>,
    shared_r_on: f64,
    x_initial: Option<&[f64]>,
    t_final: f64,
    step: TimeStep,
    max_adaptive_steps: usize,
) -> Result<Vec<TransientWithBlocksStep>, DaeError> {
    let mut trace = Vec::new();
    simulate_transient_with_blocks_streamed(
        source,
        dialect,
        diodes,
        ideal_switches,
        blocks,
        gates,
        shared_r_on,
        x_initial,
        t_final,
        step,
        max_adaptive_steps,
        |t, point, outputs| {
            trace.push((t, point, outputs));
            Ok(())
        },
    )?;
    Ok(trace)
}

/// The actual transient engine — every other `simulate_transient_with_blocks*` function is a
/// thin wrapper over this one. Calls `on_step(t, point, outputs)` once per *accepted* step, in
/// order, and holds nothing beyond one step's own data itself: a caller that only needs summary
/// statistics, or that writes each row straight to a file/stream as it arrives, never pays for a
/// full in-memory trace the way `simulate_transient_with_blocks`/`_capped` do by design (they
/// exist for exactly the callers — mostly this project's own tests — that want the whole
/// `Vec` back). `on_step` returning `Err` aborts the run immediately with that error.
#[allow(clippy::too_many_arguments)]
pub fn simulate_transient_with_blocks_streamed(
    source: &str,
    dialect: Dialect,
    diodes: &BTreeMap<String, IdealDiode>,
    ideal_switches: &BTreeMap<String, IdealSwitch>,
    blocks: &[BlockInstance],
    gates: &BTreeMap<String, GateBinding>,
    shared_r_on: f64,
    x_initial: Option<&[f64]>,
    t_final: f64,
    step: TimeStep,
    max_adaptive_steps: usize,
    mut on_step: impl FnMut(f64, OperatingPoint, BTreeMap<String, SignalValue>) -> Result<(), DaeError>,
) -> Result<(), DaeError> {
    let statements = general_mna::parse_and_flatten(source, dialect).map_err(DaeError::Parse)?;

    // A CScript, CoordinateTransform, or Pmsm block's own `.name` is only its *primary* output
    // alias; `output_names` may also register extra named outputs (see evaluate_blocks' arms
    // for all three) that a gate binding is just as free to reference directly -- all need to
    // count as "known" here, or a valid netlist referencing one of those extra names gets
    // rejected before it ever runs.
    let block_names = block_index_by_name(blocks)?;

    // Structural check first: a `Sig2Phys` converter is a reference *target*, never a device with
    // terminals, so wiring one into the netlist as a node is a model error that must be reported
    // before anything else is examined -- otherwise it solves silently to `V(<name>) = 0`.
    reject_sig2phys_wired_into_circuit(&statements, blocks)?;

    for (switch_name, binding) in gates {
        for needed in binding.source_blocks().into_iter().flatten() {
            let Some(&idx) = block_names.get(needed) else {
                return Err(DaeError::UnknownBlockInput(needed.to_string()));
            };
            // A GateBinding's target must be an explicit domain=voltage Sig2Phys converter,
            // never a raw control block directly and never a domain=current Sig2Phys either --
            // an ideal switch's gate is itself a voltage, so it shares the same Signal-to-PS
            // boundary a V-source's own magnitude uses (see BlockKind::Sig2Phys's own doc
            // comment); no separate gate-only converter type exists. This also correctly
            // rejects naming a CScript/CoordinateTransform/Pmsm block's own *extra* output
            // alias directly (block_names maps those to the same index, whose kind is never
            // Sig2Phys), so no separate check is needed for that case.
            if !matches!(
                blocks[idx].kind,
                BlockKind::Sig2Phys {
                    domain: PhysicalDomain::Voltage
                }
            ) {
                return Err(DaeError::GateTargetNotSig2Voltage {
                    gate: switch_name.clone(),
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
    // real behavioral difference from `simulate_transient_with_ideal_switches` for the plain
    // fixed/PWM case (caught by exactly that mismatch: this function must reduce to identical
    // numbers as that one whenever no block reads a `BlockKind::Phys2Sig`).
    let initial_states: BTreeMap<String, (IdealSwitch, GateState)> = ideal_switches
        .iter()
        .map(|(name, m)| (name.clone(), (*m, GateState::Off)))
        .collect();
    let (system0, _) = crate::build_with_ideal_switches(
        &statements,
        dialect,
        diodes,
        &initial_states,
        shared_r_on,
    )?;

    // Signal-to-PS enforcement for a `V`/`I` source's own literal value: `general-mna` already
    // accepts a bare symbol there (`Expression::Symbol`), stamped verbatim into that source's
    // own `input_values` entry -- a genuinely time-varying `TransientFunction` source uses this
    // too, with the symbol set to the source's *own* element name (`sym == name`, resolved via
    // `system.transient_sources`, an entirely separate mechanism this check must not confuse
    // with a block-driven source). Any *other* symbol is a block-driven source candidate: it
    // must name a declared block, and that block must be a `Sig2Phys` converter of the matching
    // domain (never a raw control block directly, and never the wrong domain) -- source element
    // names are always their own SPICE device letter (`V`/`I`) by construction, which is what
    // picks the expected domain below. V/I source stamps never depend on switch state, so
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
            Some('V') | Some('v') => (
                BlockKind::Sig2Phys {
                    domain: PhysicalDomain::Voltage,
                },
                "sig2phys(domain=voltage)",
            ),
            Some('I') | Some('i') => (
                BlockKind::Sig2Phys {
                    domain: PhysicalDomain::Current,
                },
                "sig2phys(domain=current)",
            ),
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
    #[cfg(feature = "python")]
    let mut pyblock_registry = pyblock_ffi::PyBlockRegistry::new();
    #[cfg(feature = "python")]
    let mut pyfunction_registry = pyblock_ffi::PyFunctionRegistry::new();
    // Spawned lazily, the first time a BlockKind::OctFunc block is actually encountered below --
    // see octave_ffi's own module doc comment for why paying octave-cli's own real startup cost
    // (~158 ms, measured) should never happen for a run that doesn't use kind=octfunc at all.
    // Shared (via Rc::clone into every BlockState::OctFunction instance) rather than one process
    // per block instance -- this is the stateless/pyfunc-style case, so every octfunc block in
    // one run talks to the same octave-cli session.
    let mut octave_session: Option<std::rc::Rc<std::cell::RefCell<octave_ffi::OctaveSession>>> =
        None;
    // Which directories have already been addpath'd into the shared session -- addpath itself
    // is harmless to repeat, but there's no reason to pay the round trip twice for two block
    // instances whose .m files happen to live in the same directory.
    let mut octave_paths_added: std::collections::BTreeSet<std::path::PathBuf> =
        std::collections::BTreeSet::new();
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
            BlockKind::DiscreteStateSpace { ss, sample_time } => {
                let (period, offset) = match sample_time {
                    SampleTimeSpec::Periodic { period, offset } => (*period, *offset),
                    SampleTimeSpec::Variable => unreachable!(
                        "general-mna's parse_required_periodic_sample_time already rejected \
                         ts=variable for kind=discretestatespace"
                    ),
                };
                BlockState::DiscreteDynamic {
                    x: vec![0.0; ss.states()],
                    last_output: vec![0.0; ss.outputs()],
                    // Same `period - offset` convention as BlockKind::CScript's own init below
                    // -- guarantees the first evaluate_blocks call is due at t=offset.
                    time_since_sample: period - offset,
                    state_space: ss.clone(),
                }
            }
            BlockKind::DiscreteTransferFunction { tf, sample_time } => {
                let (period, offset) = match sample_time {
                    SampleTimeSpec::Periodic { period, offset } => (*period, *offset),
                    SampleTimeSpec::Variable => unreachable!(
                        "general-mna's parse_required_periodic_sample_time already rejected \
                         ts=variable for kind=discretetf"
                    ),
                };
                let state_space = tf.to_state_space();
                BlockState::DiscreteDynamic {
                    x: vec![0.0; state_space.states()],
                    last_output: vec![0.0],
                    time_since_sample: period - offset,
                    state_space,
                }
            }
            BlockKind::DiscretePid { sample_time, .. } => {
                let (period, offset) = match sample_time {
                    SampleTimeSpec::Periodic { period, offset } => (*period, *offset),
                    SampleTimeSpec::Variable => unreachable!(
                        "general-mna's parse_required_periodic_sample_time already rejected \
                         ts=variable for kind=discretepid"
                    ),
                };
                BlockState::DiscretePid {
                    state: DiscretePidState::default(),
                    time_since_sample: period - offset,
                    last_output: 0.0,
                }
            }
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
            // LogicGate needs no BlockState at all -- purely combinational, falls through to
            // the `_ => BlockState::Stateless` catch-all below, same as Sum/Gain.
            BlockKind::SrLatch { .. } => BlockState::SrLatch { q: false },
            // prev_clk starts at 0.0 (clk below threshold) -- matches every other dynamic
            // block's own "starts at rest" convention, and means a clk that's already high on
            // the very first step is correctly treated as a rising edge (0.0 -> high counts),
            // not silently missed.
            BlockKind::FlipFlop { .. } => BlockState::FlipFlop {
                q: false,
                prev_clk: 0.0,
            },
            BlockKind::Counter { .. } => BlockState::Counter {
                count: 0,
                prev_clk: 0.0,
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
                if matches!(sample_time, Some(SampleTimeSpec::Variable))
                    && !instance.supports_next_sample_hit()
                {
                    return Err(
                        DaeError::CScriptRequiresNextSampleHitForVariableSampleTime {
                            block_name: b.name.clone(),
                        },
                    );
                }
                BlockState::CScript {
                    instance,
                    // Periodic{period, offset}: initialized to `period - offset` (not 0.0, and
                    // deliberately not an infinite sentinel -- that would poison the first
                    // call's `elapsed` value into +inf once `dt` is added to it below) --
                    // `offset == 0.0` (the default when `to=` is omitted) makes this `period`
                    // itself, guaranteeing the first evaluate_blocks call is always "due"
                    // (time_since_sample + dt >= period trivially) exactly like every
                    // `ts=`/`freq=`-using netlist already behaved before `to=` could exist at
                    // all; a nonzero `offset` delays that first hit to `t=offset` instead.
                    // Variable/None: `0.0` -- Variable's own threshold is `next_hit_dt` (itself
                    // seeded to `0.0` below), so this is unconditionally due on the first call
                    // too, the same convention.
                    time_since_sample: match sample_time {
                        Some(SampleTimeSpec::Periodic { period, offset }) => period - offset,
                        Some(SampleTimeSpec::Variable) | None => 0.0,
                    },
                    next_hit_dt: 0.0,
                    last_output: vec![0.0; output_names.len()],
                    // Starts at rest, matching every other dynamic block's own convention
                    // (Pid/StateSpace/TransferFunction/Pmsm/Vco all start their own state at
                    // zero too).
                    xc: vec![0.0; *xc_count],
                }
            }
            #[cfg(feature = "python")]
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
                if matches!(sample_time, Some(SampleTimeSpec::Variable))
                    && !instance.supports_next_sample_hit()
                {
                    return Err(
                        DaeError::PyBlockRequiresNextSampleHitForVariableSampleTime {
                            block_name: b.name.clone(),
                        },
                    );
                }
                BlockState::PyBlock {
                    instance,
                    // Same rationale as CScript's own construction arm above.
                    time_since_sample: match sample_time {
                        Some(SampleTimeSpec::Periodic { period, offset }) => period - offset,
                        Some(SampleTimeSpec::Variable) | None => 0.0,
                    },
                    next_hit_dt: 0.0,
                    last_output: vec![0.0; output_names.len()],
                    xc: vec![0.0; *xc_count],
                }
            }
            // A netlist declaring kind=pyblock against a build with the `python` feature off --
            // see DaeError::PythonSupportNotCompiledIn's own doc comment.
            #[cfg(not(feature = "python"))]
            BlockKind::PyBlock { .. } => {
                return Err(DaeError::PythonSupportNotCompiledIn {
                    block_name: b.name.clone(),
                });
            }
            #[cfg(feature = "python")]
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
                    // Variable is unreachable here -- rejected at parse time for PyFunction
                    // (see BlockKind::PyFunction's own doc comment) -- so only Periodic/None
                    // ever appear; same rationale as CScript's own construction arm above.
                    time_since_sample: match sample_time {
                        Some(SampleTimeSpec::Periodic { period, offset }) => period - offset,
                        Some(SampleTimeSpec::Variable) | None => 0.0,
                    },
                    last_output: vec![0.0; output_names.len()],
                }
            }
            #[cfg(not(feature = "python"))]
            BlockKind::PyFunction { .. } => {
                return Err(DaeError::PythonSupportNotCompiledIn {
                    block_name: b.name.clone(),
                });
            }
            BlockKind::OctFunc {
                path,
                output_names,
                sample_time,
                ..
            } => {
                // Spawn the shared session on first use, not eagerly -- see its own `let mut
                // octave_session` declaration above.
                let session = match &octave_session {
                    Some(s) => s.clone(),
                    None => {
                        let s = octave_ffi::OctaveSession::spawn().map_err(DaeError::Octave)?;
                        let s = std::rc::Rc::new(std::cell::RefCell::new(s));
                        octave_session = Some(s.clone());
                        s
                    }
                };
                // Octave requires the .m file to be on its own path and the file name to match
                // the function name (see octave_ffi's own module doc comment) -- addpath once
                // per unique containing directory, not once per block instance.
                let dir = match path.canonicalize() {
                    Ok(abs_file) => abs_file
                        .parent()
                        .map(std::path::Path::to_path_buf)
                        .unwrap_or_else(|| std::path::PathBuf::from(".")),
                    // A .m file that doesn't (yet) resolve via canonicalize -- fall back to its
                    // own literal parent, relative to this process's own current directory
                    // (which octave-cli inherits unchanged, since it's spawned with no explicit
                    // current_dir override). Calling into a function this session genuinely
                    // can't find then surfaces naturally as an ordinary Octave-side "undefined
                    // function" DaeError::Octave(OctaveError::Runtime) at call time, rather than
                    // failing here at construction time.
                    Err(_) => path
                        .parent()
                        .map(std::path::Path::to_path_buf)
                        .unwrap_or_else(|| std::path::PathBuf::from(".")),
                };
                if octave_paths_added.insert(dir.clone()) {
                    session
                        .borrow_mut()
                        .add_path(&dir)
                        .map_err(DaeError::Octave)?;
                }
                BlockState::OctFunction {
                    session,
                    // Same rationale as PyFunction's own construction arm above.
                    time_since_sample: match sample_time {
                        Some(SampleTimeSpec::Periodic { period, offset }) => period - offset,
                        Some(SampleTimeSpec::Variable) | None => 0.0,
                    },
                    last_output: vec![0.0; output_names.len()],
                }
            }
            BlockKind::OctBlock {
                path,
                function,
                output_names,
                sample_time,
                xc_count,
            } => {
                // OctBlock's own per-instance state genuinely lives inside the shared Octave
                // session (see BlockState::OctBlock's own doc comment) -- a rejected trial step
                // under TimeStep::Adaptive would silently persist its own state-mutating calls
                // with no way to roll them back, since block_states.clone() never clones the
                // Octave-side state at all. Unconditional restriction; see
                // DaeError::OctBlockDoesNotSupportAdaptiveStep's own doc comment.
                if matches!(step, TimeStep::Adaptive(_)) {
                    return Err(DaeError::OctBlockDoesNotSupportAdaptiveStep {
                        block_name: b.name.clone(),
                    });
                }
                // Spawn the shared session on first use, not eagerly -- same shared session
                // every other Octave-hosted block kind in this run uses (see OctFunc's own
                // construction arm above for the full rationale).
                let session = match &octave_session {
                    Some(s) => s.clone(),
                    None => {
                        let s = octave_ffi::OctaveSession::spawn().map_err(DaeError::Octave)?;
                        let s = std::rc::Rc::new(std::cell::RefCell::new(s));
                        octave_session = Some(s.clone());
                        s
                    }
                };
                // Unlike OctFunc's own `path` (a single .m file), OctBlock's `path` is already
                // the directory containing every `<function>_<role>.m` file (see
                // BlockKind::OctBlock's own doc comment, the file-per-function table) -- addpath
                // this directory itself, once per unique directory, not once per instance.
                let dir = match path.canonicalize() {
                    Ok(abs_dir) => abs_dir,
                    // A directory that doesn't (yet) resolve via canonicalize -- fall back to
                    // its own literal path, relative to this process's own current directory.
                    // Any file this session genuinely can't find then surfaces as a clear
                    // OctBlockMissingRequiredFile error below (checked directly against the
                    // filesystem, not deferred to Octave's own "undefined function").
                    Err(_) => path.clone(),
                };
                let required = |role: &str| -> Result<(), DaeError> {
                    let expected_path = dir.join(format!("{function}_{role}.m"));
                    if expected_path.is_file() {
                        Ok(())
                    } else {
                        Err(DaeError::OctBlockMissingRequiredFile {
                            block_name: b.name.clone(),
                            expected_path,
                        })
                    }
                };
                // <function>_start.m is always required.
                required("start")?;
                if *xc_count > 0 {
                    // The xc_count > 0 contract: <function>_derivative.m/_output_xc.m instead
                    // of the plain <function>.m -- same split CScript's own xc_count has.
                    required("derivative")?;
                    required("output_xc")?;
                } else {
                    // The plain contract's own output function is named exactly `<function>.m`,
                    // not `<function>_output.m` -- checked directly rather than via the
                    // `required` closure above (which always appends a `_<role>` suffix).
                    let expected_path = dir.join(format!("{function}.m"));
                    if !expected_path.is_file() {
                        return Err(DaeError::OctBlockMissingRequiredFile {
                            block_name: b.name.clone(),
                            expected_path,
                        });
                    }
                }
                if matches!(sample_time, Some(SampleTimeSpec::Variable))
                    && !dir.join(format!("{function}_next_sample_hit.m")).is_file()
                {
                    return Err(
                        DaeError::OctBlockRequiresNextSampleHitForVariableSampleTime {
                            block_name: b.name.clone(),
                        },
                    );
                }
                let has_update = dir.join(format!("{function}_update.m")).is_file();

                if octave_paths_added.insert(dir.clone()) {
                    session
                        .borrow_mut()
                        .add_path(&dir)
                        .map_err(DaeError::Octave)?;
                }
                // Initialize this instance's own state slot once, at construction -- the same
                // "first-touch" convention PyBlock's own instantiate() already uses (calling
                // `start()` immediately, not deferred to the first evaluate_blocks call).
                session
                    .borrow_mut()
                    .call_start(function, &b.name)
                    .map_err(DaeError::Octave)?;
                BlockState::OctBlock {
                    session,
                    instance: b.name.clone(),
                    // Same rationale as CScript's/PyBlock's own construction arms above.
                    time_since_sample: match sample_time {
                        Some(SampleTimeSpec::Periodic { period, offset }) => period - offset,
                        Some(SampleTimeSpec::Variable) | None => 0.0,
                    },
                    next_hit_dt: 0.0,
                    last_output: vec![0.0; output_names.len()],
                    xc: vec![0.0; *xc_count],
                    has_update,
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

    let mut accepted_steps: usize = 0;
    let mut t = 0.0;

    match step {
        TimeStep::Fixed(dt) => {
            let steps = (t_final / dt).round() as usize;
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
                let states: BTreeMap<String, (IdealSwitch, GateState)> = ideal_switches
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

                let (system, all_diodes) = crate::build_with_ideal_switches(
                    &statements,
                    dialect,
                    diodes,
                    &states,
                    shared_r_on,
                )?;
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
                on_step(t, point, outputs)?;
            }
        }
        TimeStep::Adaptive(config) => {
            let mut dt_next = config.dt_init;
            let mut step_index = 0usize;
            while t < t_final {
                let mut dt = dt_next.min(t_final - t);
                // Clamp to the earliest predicted discrete-block/gate-edge event so a trial step
                // lands (approximately) exactly on it, instead of stepping past it and only
                // detecting the change after the fact via `gate_changed` -- the fix for silently
                // missed edges when `dt_max` approaches or exceeds the switching period (see
                // `earliest_next_event_dt`'s own doc comment). This can only shrink `dt` here;
                // the inner LTE-retry loop below may shrink it further via
                // `attempt.suggested_dt_next` on rejection, but never grows it back past this
                // clamp within the same outer iteration. Landing exactly on a predicted event
                // takes priority over `config.dt_min` -- missing an edge is worse than one small
                // out-of-target step; the `1e-15` floor just avoids a zero/negative-dt retry loop
                // from float noise when we're already sitting on the event.
                if let Some(event_dt) =
                    earliest_next_event_dt(blocks, &block_states, &prev_outputs, t)
                {
                    dt = dt.min(event_dt.max(1e-15));
                }
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
                    let states: BTreeMap<String, (IdealSwitch, GateState)> = ideal_switches
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

                    let (system, all_diodes) = crate::build_with_ideal_switches(
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
                    on_step(t, attempt.point, outputs)?;
                    accepted_steps += 1;
                    step_index += 1;
                    if accepted_steps > max_adaptive_steps {
                        return Err(DaeError::AdaptiveStepStalled {
                            steps_taken: accepted_steps,
                            t_reached: t,
                            t_final,
                        });
                    }
                    break;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod next_pwm_edge_dt_tests {
    use super::next_pwm_edge_dt;

    /// The exact bug this guards against: once the adaptive loop lands a step precisely on a
    /// threshold (the whole point of `earliest_next_event_dt`), `theta_now == threshold` to
    /// float precision on the very next call. Naively that reads as "0 away," which an outer
    /// `.max(1e-15)` clamp turns into a real near-zero step instead of an error -- and since
    /// theta barely advances in a step that small, the SAME threshold reads as ~0 again next
    /// call too: an unbounded near-zero-dt stall that grows the in-memory row buffer without
    /// limit until the process (and, in the real run that found this, the whole machine) runs
    /// out of memory. Asserts the fixed behavior directly: landing exactly on `duty` must report
    /// "a full period away," not "already there."
    #[test]
    fn landing_exactly_on_a_threshold_reports_a_full_period_not_zero() {
        let freq = 600_000.0;
        let duty = 0.4;
        let red_frac = 0.0;
        let fed_frac = 0.0;
        let dt = next_pwm_edge_dt(duty, duty, red_frac, fed_frac, freq)
            .expect("finite freq must always report a next-edge estimate");
        let half_period = 0.5 / freq;
        assert!(
            dt > half_period,
            "landing exactly on duty's own threshold must report ~a full period away \
             ({half_period:.3e}s expected), not ~0 (got {dt:.3e}s) -- ~0 is exactly the bug \
             that stalled the adaptive loop and exhausted memory on a real run",
        );
    }

    /// Same case for the `red_frac` threshold (the other edge `Pwm`/`PhaseShiftPwm` compare
    /// against), landed a few `f64::EPSILON`-scale ULPs *past* the exact instant (real
    /// floating-point accumulation can produce this just as easily as landing exactly on it) --
    /// with `duty` set to the SAME threshold so there's only one edge in play, isolating this
    /// from the (correct, separately-tested) case where a genuinely different, nearer threshold
    /// exists.
    #[test]
    fn landing_a_few_ulps_past_a_threshold_also_reports_a_full_period_not_zero() {
        let freq = 300_000.0;
        let red_frac = 0.1;
        let theta_now = red_frac + 4.0 * f64::EPSILON;
        let dt = next_pwm_edge_dt(theta_now, red_frac, red_frac, 0.0, freq)
            .expect("finite freq must always report a next-edge estimate");
        let half_period = 0.5 / freq;
        assert!(
            dt > half_period,
            "landing a few ULPs past red_frac must report ~a full period away \
             ({half_period:.3e}s expected), not ~0 (got {dt:.3e}s)",
        );
    }

    /// Normal, non-degenerate cases stay essentially unchanged: a threshold genuinely ahead of
    /// `theta_now` reports (approximately) the real distance to it, not a full period.
    #[test]
    fn a_threshold_genuinely_ahead_reports_the_real_distance() {
        let freq = 100_000.0; // period = 10us
        let theta_now = 0.1;
        let duty = 0.4; // 0.3 of a period ahead
        let dt = next_pwm_edge_dt(theta_now, duty, 0.0, 0.0, freq).unwrap();
        let expected = 0.3 / freq;
        assert!(
            (dt - expected).abs() < 1e-9 / freq,
            "expected ~{expected:.3e}s, got {dt:.3e}s"
        );
    }
}
