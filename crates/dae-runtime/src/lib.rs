//! Assembles a circuit's linear MNA system (via `general-mna`, including its `'D'` stamp) with
//! PWL device segments (via `pwl-devices`) into one Linear Complementarity Problem, solves it
//! (via `lcp-solver`), and reports the resolved DC operating point. No Newton-Raphson, no
//! voltage limiting, anywhere in this crate — see this repository's `docs/architecture.md`.
//!
//! ## The generic Thevenin/LCP fold
//!
//! `general-mna` stamps each diode `k` as a *fixed* conductance `{k}_G` plus a per-instance
//! current-source symbol `{k}_Ioff`. Fixing `{k}_G` at the diode's canonical reference slope
//! `g_off` (see `pwl_devices::IdealDiode::canonical`) makes the whole linear system `A0` genuinely
//! fixed — independent of which segment every diode ends up in — with every segment's actual
//! nonlinearity pushed entirely into the `{k}_Ioff` terms via each diode's canonical
//! `max(0, ...)` (`z`) decomposition. Since the system is linear in those `Ioff` terms, each
//! diode's terminal voltage is an affine function of every diode's `z` variables:
//!
//! ```text
//! x(z)   = x0 + sum_j w_j * raw_Ioff_j                  (raw_Ioff_j = delta_on_j*z2_j - delta_br_j*z1_j)
//! V_k(z) = v0_k + sum_j gamma_kj * raw_Ioff_j            (gamma_kj = w_j[p_k] - w_j[n_k])
//! ```
//!
//! where `x0 = solve(A0, u0)` is the baseline operating point with every diode's `Ioff = 0`,
//! and `w_j = solve(A0, B[:, diode j's input column])` is how much a unit "raw Ioff" on diode
//! `j` moves every unknown. Substituting `V_k(z)` into each diode's guard equations
//! (`w1_k = V_k - v_breakdown_k + z1_k`, `w2_k = v_th_k - V_k + z2_k`) gives exactly the LCP
//! `(M, q)` this crate builds and hands to `lcp_solver::solve`.

mod block_graph;
mod closed_loop;
mod linsolve;
mod step_control;
mod topology;

pub use block_graph::{
    reject_sig2phys_wired_into_circuit, simulate_transient_with_blocks,
    simulate_transient_with_blocks_capped, simulate_transient_with_blocks_streamed, BlockInstance,
    BlockKind, ConstValue, GainValue, GateBinding, Phys2SigTarget, PhysicalDomain, PidClamp,
    SampleTimeSpec, Signal, SignalValue, TransientWithBlocksStep, ADAPTIVE_STEP_HARD_CAP,
};
pub use closed_loop::{sawtooth_carrier, simulate_closed_loop};
pub use step_control::{AdaptiveConfig, TimeStep};

use std::collections::BTreeMap;

use general_mna::{
    BuildError, BuildOptions, EvaluationError, Expression, InitialStateError, MnaBuilder, MnaSystem,
};
use general_spice_core::ast::Statement;
use general_spice_core::Dialect;
use lcp_solver::LcpError;
use linsolve::{dense_solve, SingularMatrix};
use pwl_devices::{IdealDiode, IdealSwitch};

pub use general_mna::SwitchState as GateState;
pub use general_mna::TransientFunction;

#[derive(Debug, Clone, PartialEq)]
pub struct OperatingPoint {
    pub unknowns: Vec<String>,
    pub x: Vec<f64>,
    /// Resolved `(z1, z2)` for each diode, in the same iteration order as the `diodes` map
    /// passed to [`solve_dc`] (a `BTreeMap`, so alphabetical by name) — useful for reporting
    /// which segment each diode ended up in.
    pub diode_names: Vec<String>,
    pub diode_z: Vec<(f64, f64)>,
    /// Each diode's true (unscaled) resolved current `delta_on*z2 - delta_br*z1`, same order
    /// as `diode_names`/`diode_z`. Needed as history by [`simulate_transient`]'s trapezoidal
    /// step (see its doc comment); exposed here rather than recomputed by every caller.
    pub diode_raw_ioff: Vec<f64>,
}

impl OperatingPoint {
    pub fn value(&self, unknown: &str) -> Option<f64> {
        self.unknowns
            .iter()
            .position(|name| name == unknown)
            .map(|index| self.x[index])
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DaeError {
    /// Parsing/hierarchy-flattening the netlist itself failed — see
    /// [`general_mna::parse_and_flatten`]; a `.subckt`/`X` structural error (undefined
    /// subcircuit, node-count mismatch, a recursive instantiation, ...) surfaces here, distinct
    /// from [`DaeError::Build`]'s "parsed fine, but this statement has no valid electrical
    /// stamp" class of error.
    Parse(String),
    /// A streaming `on_step` callback's own I/O failed (writing a CSV/raw row straight to a
    /// file/stream as it's produced — see [`block_graph::simulate_transient_with_blocks_streamed`]).
    /// Carries the formatted underlying `std::io::Error` as a string rather than the error type
    /// itself, since `DaeError` derives `PartialEq` and `io::Error` doesn't implement it.
    Io(String),
    Build(BuildError),
    /// The netlist declares `ic=` initial conditions that could not be turned into a consistent
    /// starting state — see [`general_mna::MnaSystem::initial_state`], which does that solve and
    /// whose own error explains the two ways it fails (contradictory conditions, or a node with
    /// no DC path once every `ic`-free capacitor is opened and inductor shorted).
    InitialCondition(InitialStateError),
    Evaluate(EvaluationError),
    Linear(SingularMatrix),
    Lcp(LcpError),
    UnknownDiodeInput(String),
    /// A [`block_graph::BlockInstance`]'s input, or a [`block_graph::GateBinding`], names a
    /// block that doesn't exist anywhere in the graph (a typo, or a genuinely undeclared name)
    /// — declaration *position* is no longer a possible cause: [`block_graph::topological_order`]
    /// derives each step's evaluation order from the `Signal::Block` dependency graph itself, so
    /// a block may reference another declared anywhere in the same slice, before or after it.
    UnknownBlockInput(String),
    /// The `Signal::Block` dependency graph among a [`block_graph::BlockInstance`] slice
    /// contains a genuine same-step cycle — computed once, upfront, by
    /// [`block_graph::topological_order`] (a DFS-based topological sort; a cycle shows up as a
    /// back-edge into a node still on the current recursion path) before any step is solved.
    /// `cycle` is the exact closing path, block names in dependency order with the first name
    /// repeated at the end to show the loop closing (e.g. `["A", "B", "C", "A"]`); a
    /// self-referencing block reports `["A", "A"]`. Fix by inserting a `Signal::BlockPrev`
    /// somewhere in the cycle — the one-sample delay that turns a same-step algebraic loop into
    /// a legitimate sampled-data feedback path.
    AlgebraicLoop {
        cycle: Vec<String>,
    },
    /// Loading or resolving symbols in a [`block_graph::BlockKind::CScript`]'s shared library
    /// failed — see [`cscript_ffi::CScriptError`].
    CScript(cscript_ffi::CScriptError),
    /// A netlist declares a [`block_graph::BlockKind::CScript`] block whose library doesn't
    /// export `cscript_clone`, but the run was requested with [`TimeStep::Adaptive`], which
    /// needs to clone every block's state before each trial step — see `cscript_ffi`'s own
    /// module doc comment, "Adaptive step-size control and instance cloning." Pass a fixed
    /// `TimeStep::Fixed` instead, or add `cscript_clone` to the library.
    CScriptRequiresCloneForAdaptiveStep {
        block_name: String,
    },
    /// A netlist declares a [`block_graph::BlockKind::CScript`] block with `ts=variable`
    /// (`block_graph::SampleTimeSpec::Variable`), but its library doesn't export
    /// `cscript_next_sample_hit` — the required function for a block that computes its own
    /// schedule; see `cscript_ffi`'s own module doc comment, "The optional block-controlled
    /// sample time." Checked once, up front, the same "checked once, up front" discipline
    /// `CScriptRequiresCloneForAdaptiveStep` already uses.
    CScriptRequiresNextSampleHitForVariableSampleTime {
        block_name: String,
    },
    /// The `PyBlock` counterpart to [`Self::CScriptRequiresNextSampleHitForVariableSampleTime`]
    /// — a `.py` file declared `ts=variable` but doesn't define `next_sample_hit`.
    #[cfg(feature = "python")]
    PyBlockRequiresNextSampleHitForVariableSampleTime {
        block_name: String,
    },
    /// Loading, calling into, or cloning a [`block_graph::BlockKind::PyBlock`]'s own `.py` file
    /// failed — see [`pyblock_ffi::PyBlockError`]. Unlike [`Self::CScriptRequiresCloneForAdaptiveStep`],
    /// there is no separate "doesn't support adaptive stepping" variant for `PyBlock`: cloning
    /// uses Python's own generic `copy.deepcopy`, which needs no author opt-in, so a `PyBlock`
    /// is always eligible — a genuine failure to deep-copy its own state instead surfaces here,
    /// as an ordinary [`pyblock_ffi::PyBlockError::Exception`].
    #[cfg(feature = "python")]
    PyBlock(pyblock_ffi::PyBlockError),
    /// A netlist declares a [`block_graph::BlockKind::PyBlock`]/[`block_graph::BlockKind::
    /// PyFunction`] block, but this build of `dae-runtime` was compiled without the optional
    /// `python` feature (which pulls in `pyblock-ffi`, and through it a build-time dependency
    /// on a discoverable Python/libpython) — unconditional, unlike the two variants above,
    /// specifically so a build without that feature can still report a clear, actionable error
    /// instead of failing to compile at all or panicking.
    PythonSupportNotCompiledIn {
        block_name: String,
    },
    /// Spawning the shared `octave-cli` subprocess, adding a [`block_graph::BlockKind::OctFunc`]
    /// block's own `.m` file directory to its path, or calling into it failed — see
    /// [`octave_ffi::OctaveError`]'s own three cases (not found on `PATH`, the process died, or
    /// an ordinary Octave-side runtime error the session survives). Unlike
    /// [`Self::PythonSupportNotCompiledIn`], there is no "not compiled in" counterpart for this
    /// variant — `octave-ffi` is an unconditional dependency of this crate (a pure subprocess
    /// manager, never linking against Octave itself), so `kind=octfunc` is always available to
    /// parse and construct; only an actually-missing `octave-cli` binary on `PATH` at runtime
    /// surfaces here, as [`octave_ffi::OctaveError::NotFound`].
    Octave(octave_ffi::OctaveError),
    /// A [`block_graph::BlockKind::OctBlock`]'s own `path=` directory is missing one of the
    /// `.m` files its declared contract requires (see that variant's own doc comment for the
    /// full file-per-function table) — checked once, up front at construction, the same
    /// "checked once, up front" discipline [`Self::CScriptRequiresCloneForAdaptiveStep`]
    /// already uses, and deliberately *before* ever spawning `octave-cli` for this block, since
    /// the filesystem already has the answer without needing a round trip through Octave's own
    /// "undefined function" error.
    OctBlockMissingRequiredFile {
        block_name: String,
        expected_path: std::path::PathBuf,
    },
    /// A netlist declares a [`block_graph::BlockKind::OctBlock`] block with `ts=variable`
    /// (`block_graph::SampleTimeSpec::Variable`), but `<function>_next_sample_hit.m` doesn't
    /// exist in its own `path=` directory — the `OctBlock` counterpart to
    /// [`Self::CScriptRequiresNextSampleHitForVariableSampleTime`]/
    /// [`Self::PyBlockRequiresNextSampleHitForVariableSampleTime`].
    OctBlockRequiresNextSampleHitForVariableSampleTime {
        block_name: String,
    },
    /// A netlist declares a [`block_graph::BlockKind::OctBlock`] block, but the run was
    /// requested with [`TimeStep::Adaptive`], which clones the whole `block_states` vector
    /// before every trial step and simply discards a rejected trial's clone (see
    /// [`Self::CScriptRequiresCloneForAdaptiveStep`]'s own doc comment). That protocol depends
    /// on a block's own state living entirely inside the cloned Rust value -- true for
    /// `CScript`/`PyBlock` (an owned, independently-cloned native/Python state), but **not**
    /// true for `OctBlock`: its own opaque per-instance state lives inside the one shared
    /// `octave-cli` session itself (see [`block_graph::BlockState::OctBlock`]'s own doc
    /// comment), which is never cloned, so a rejected trial's own state-mutating calls
    /// (`output`/`output_xc`/`update`) would silently persist in the real Octave-side state
    /// with no way to roll them back. Unlike `CScriptRequiresCloneForAdaptiveStep` (opt-in,
    /// satisfiable by exporting `cscript_clone`), this restriction is unconditional -- there is
    /// no `.m`-file convention that could make Octave's own shared global-struct state
    /// trial-cloneable. Use a fixed `TimeStep::Fixed` for any netlist using `kind=octblock`.
    OctBlockDoesNotSupportAdaptiveStep {
        block_name: String,
    },
    /// [`TimeStep::Adaptive`] took more than [`ADAPTIVE_STEP_HARD_CAP`] accepted steps without
    /// reaching `t_final` — a hard safety ceiling, not a normal error a well-behaved run should
    /// ever hit. Exists because the adaptive loop accumulates one row per accepted step into an
    /// in-memory trace with no other bound, and a genuinely stalled step controller (e.g. a
    /// near-zero-`dt` loop that never makes real forward progress — the exact class of bug
    /// `next_pwm_edge_dt`'s own `THETA_EPS` guards against, see its doc comment) would otherwise
    /// grow that trace without limit until the process — and, on the real run that first hit
    /// this, the whole machine — ran out of memory. `steps_taken` is how many were accepted
    /// before the cap tripped; `t_reached`/`t_final` show how far the run actually got.
    AdaptiveStepStalled {
        steps_taken: usize,
        t_reached: f64,
        t_final: f64,
    },
    /// A [`block_graph::GateBinding`] names a block that exists but isn't a `domain=voltage`
    /// [`block_graph::BlockKind::Sig2Phys`] — the enforced physical/signal-domain boundary:
    /// an ideal switch's gate is itself a voltage, so any signal driving it must first pass through
    /// the same `domain=voltage` `Sig2Phys` converter a `V`-source's own magnitude uses, never a
    /// raw `Pid`/`Vco`/`Hysteresis`/etc. block directly, and never a `domain=current` `Sig2Phys`
    /// either. `gate` is the ideal switch this binding belongs to; `block` and `found_kind` name
    /// the offending target and (for a human-readable message) what it actually is.
    GateTargetNotSig2Voltage {
        gate: String,
        block: String,
        found_kind: &'static str,
    },
    /// An independent `V`/`I` source's own literal value in the netlist is a bare symbol that
    /// names a declared block, but that block isn't a [`block_graph::BlockKind::Sig2Phys`] of
    /// the matching domain (`V` needs `domain=voltage`, `I` needs `domain=current`) — the
    /// enforced Signal-to-PS boundary for driving a source's own magnitude from the block graph.
    SourceNotSig2PhysicalConverter {
        source: String,
        block: String,
        expected_kind: &'static str,
        found_kind: &'static str,
    },
    /// A [`block_graph::BlockKind::Sig2Phys`] converter's own name is also used as an ordinary
    /// circuit node on an element line — i.e. the converter was *wired into* the physical
    /// network instead of being *referenced by name* from the one place that can consume it.
    ///
    /// A `kind=sig2phys` block has no terminals and stamps nothing into the MNA system; it is
    /// purely a named Signal-to-PS value another statement substitutes. So a net that happens to
    /// share its name is just an ordinary, otherwise-undriven node: its KCL row is solved
    /// consistently at `0` (a resistor to ground is a perfectly non-singular
    /// `G·v = 0`) and the run silently reports `V(<name>) = 0` next to the block's own,
    /// correct, non-zero output column. That silent wrong answer is exactly what this variant
    /// exists to turn into a build-time error, before any step is solved.
    ///
    /// The two legitimate ways to consume a `Sig2Phys` are both by name, never by wire:
    /// an independent `V`/`I` source's own value field (`V1 a 0 VDRV`, see
    /// [`DaeError::SourceNotSig2PhysicalConverter`]) and an ideal switch's `gate=`/`ctrl=` field
    /// (see [`DaeError::GateTargetNotSig2Voltage`]).
    ///
    /// `block` is the converter as declared, `element` the first element line that wires it, and
    /// `node` that element's node token as written. Node names are matched case-insensitively,
    /// matching how the netlist grammar itself treats them (`A` and `a` are one node).
    Sig2PhysUsedAsCircuitNode {
        block: String,
        element: String,
        node: String,
    },
    /// Two blocks in the same slice resolve to the same name — either two
    /// [`block_graph::BlockInstance`]s declared with the same `.name`, or one block's own
    /// extra `output_names` entry (a [`block_graph::BlockKind::CScript`]/`CoordinateTransform`/
    /// `Pmsm`/`Pwm`/`PhaseShiftPwm`'s secondary output alias) colliding with another block's
    /// name. Without this check, `block_index_by_name`'s `BTreeMap::insert` would silently let
    /// the later one win, so a `Signal::Block(name)` reference would silently resolve to the
    /// wrong block instead of failing loudly.
    DuplicateBlockName(String),
    /// A [`block_graph::BlockKind`] that requires a `SignalValue::Scalar` input (or a specific
    /// operand shape, e.g. `Sum`/`Product`'s "all-scalar or all-vector, never mixed" rule, or a
    /// matrix `Gain`'s own required vector length) received a `SignalValue::Vector` it can't
    /// accept — the default, fail-closed rule for any block kind without an elementwise/
    /// broadcast rule of its own. See `book/dev-guide/src/vector-signals.md` for exactly which
    /// block kinds accept a `Vector` and under what rule.
    VectorSignalNotSupported {
        block: String,
    },
    /// Two (or more) `SignalValue::Vector`s feeding the same block disagree in length (e.g.
    /// `Sum`'s own inputs, `MathFn2`'s two operands), or a `Vector`'s own length doesn't match
    /// a fixed arity the block itself declares (a matrix `Gain`'s own column count, a
    /// `StateSpace`'s own declared input count).
    VectorSignalSizeMismatch {
        block: String,
        expected: usize,
        got: usize,
    },
}

/// Solves the DC operating point of a netlist containing linear devices plus any number of
/// `D` (diode) elements, whose piecewise-linear parameters are supplied in `diodes` (keyed by
/// element name) rather than parsed from the netlist — SPICE's `.model` syntax has no notion
/// of this crate's breakdown/leakage/forward segments.
pub fn solve_dc(
    source: &str,
    dialect: Dialect,
    diodes: &BTreeMap<String, IdealDiode>,
) -> Result<OperatingPoint, DaeError> {
    let statements = general_mna::parse_and_flatten(source, dialect).map_err(DaeError::Parse)?;
    let system = MnaBuilder::new(dialect)
        .build_statements(&statements)
        .map_err(DaeError::Build)?;
    fold_and_solve(
        &system,
        &statements,
        diodes,
        &Scheme::Dc,
        0.0,
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
}

/// Which implicit integration formula a given step of [`simulate_transient`] uses. Both
/// reduce a descriptor DAE `A x + K dx/dt = B u` to the same linear-solve shape [`solve_dc`]
/// already uses (an effective matrix/RHS folded into the diode LCP unchanged) — see
/// `docs/architecture.md` and each variant's own derivation below.
enum Scheme<'a> {
    /// `A x = B u` directly (no time derivative at all) — an ordinary DC operating point.
    Dc,
    /// `dx/dt ~= (x_{n+1} - x_n) / dt`, giving `(A + K/dt) x_{n+1} = B u_{n+1} + (K/dt) x_n`.
    /// First-order accurate, but tolerant of an inconsistent `x_prev` (doesn't need `x_prev`
    /// to already satisfy the circuit's algebraic constraints exactly) — this is what makes
    /// it the right choice for the very first transient step, and immediately after a
    /// LCP-resolved mode change (see `Trapezoidal`'s doc comment for why trapezoidal is *not*
    /// safe in either of those cases).
    BackwardEuler { x_prev: &'a [f64], dt: f64 },
    /// Second-order accurate. Derived by summing the DAE at both `t_n` and `t_{n+1}` and using
    /// the trapezoidal relation `x'_n + x'_{n+1} = (2/dt)(x_{n+1} - x_n)` to eliminate the
    /// derivatives entirely:
    ///
    /// ```text
    /// (A/2)(x_n + x_{n+1}) + (K/dt)(x_{n+1} - x_n) = (B/2)(u_n + u_{n+1})
    /// => (A/2 + K/dt) x_{n+1} = (B/2)(u_n + u_{n+1}) + (K/dt - A/2) x_n
    /// ```
    ///
    /// Crucially, `B u` includes each diode's *nonlinear* current, not just the netlist's own
    /// (constant, in this crate's scope) sources — true trapezoidal accuracy requires
    /// averaging that too, not just the linear part. Since the new step's diode current
    /// contributes `(B/2) * raw_Ioff_{n+1}` rather than the full `B * raw_Ioff_{n+1}`, the
    /// LCP fold's per-diode coupling coefficients are scaled by `0.5` for this scheme (see
    /// `fold_and_solve`'s `coupling_scale`) — while the *previous* step's already-known
    /// current contributes its own `(B/2) * raw_Ioff_n` term directly to the effective RHS,
    /// via `prev_diode_raw_ioff`.
    ///
    /// This derivation assumes `x_n` already satisfies the circuit's algebraic constraints
    /// exactly (so that summing the DAE at `t_n` contributes nothing extra beyond
    /// `B u_n - A x_n = 0`) — true right after a [`Scheme::BackwardEuler`] or [`Scheme::Dc`]
    /// step, not true in general starting from an arbitrary `x_initial`. This is the DAE
    /// analog of "trapezoidal needs consistent initial conditions," and is why
    /// [`simulate_transient`] always starts with backward Euler and falls back to it whenever
    /// a diode's resolved segment changes between consecutive steps.
    ///
    /// That "constant, in this crate's scope" assumption about the netlist's own linear
    /// sources held until [`TransientFunction`](general_mna::TransientFunction) support was
    /// added: for a genuinely time-varying `V`/`I` source, `u_n != u_{n+1}` for that source's
    /// own column too, not just for diode currents — `fold_and_solve` evaluates `numeric0.u`
    /// (i.e. `B u_{n+1}`) a second time at `t_n = t_{n+1} - dt` and averages the two, exactly
    /// mirroring the diode-current averaging already described above, rather than silently
    /// reusing the same "constant source" shortcut for a source that no longer is one.
    Trapezoidal {
        x_prev: &'a [f64],
        dt: f64,
        prev_diode_raw_ioff: &'a BTreeMap<String, f64>,
    },
}

/// Runs a transient simulation of a netlist containing linear devices (including storage —
/// `C`, `L`) plus any number of `D` diodes, from `t = 0` to `t_final`, starting from
/// `x_initial` (all zero if `None` — the usual "circuit at rest, then a step/DC source turns
/// on at `t = 0`" case used by this crate's own tests). `step` picks fixed or adaptive timing
/// — see [`TimeStep`]/[`AdaptiveConfig`]; `TimeStep::Fixed(dt)` reproduces exactly the fixed-`dt`
/// behavior this function had before `TimeStep` existed.
///
/// Uses [`Scheme::Trapezoidal`] (second-order accurate, matching Xyce/SPICE's own default) for
/// most steps, but falls back to [`Scheme::BackwardEuler`] for the very first step (an
/// arbitrary `x_initial` is not guaranteed to satisfy the circuit's algebraic constraints —
/// trapezoidal needs that) and for any step where a diode's resolved segment (breakdown /
/// leakage / forward) differs from the *previous* step's — the architecture's own sanctioned
/// policy for staying robust across an LCP-resolved mode change (see `docs/architecture.md`).
/// This is a lagging, not predictive, policy: it reacts to the mode change already observed in
/// the previous step's result, rather than trying to foresee this step's mode before solving
/// it — simpler, and sufficient for the well-behaved circuits this crate targets so far.
///
/// No ideal switch/PWM support yet in the transient loop — see this crate's journal for why.
/// The `x` a transient run starts from, in `system.unknowns` order.
///
/// An explicit `x_initial` from the caller always wins — it is the more specific instruction,
/// and several of this crate's own tests hand-build one. Failing that, the netlist's own `ic=`
/// values are honored via [`general_mna::MnaSystem::initial_state`], which solves the
/// constrained operating point rather than merely assigning the declared numbers (the other
/// unknowns are not free: KCL still has to hold, and a source still has to supply whatever the
/// constrained state draws). Failing *that* — the overwhelmingly common case of a netlist with
/// no `ic=` anywhere — the circuit starts from rest, exactly as before.
///
/// The `ic=` operating point is evaluated at `t = 0` with every diode at its own off-segment
/// reference slope and zero Norton current, matching how the first real step's `base_values` is
/// built in `solve_step`.
pub(crate) fn resolve_initial_state(
    system: &MnaSystem,
    diodes: &BTreeMap<String, IdealDiode>,
    x_initial: Option<&[f64]>,
) -> Result<Vec<f64>, DaeError> {
    if let Some(x) = x_initial {
        return Ok(x.to_vec());
    }
    let mut values = BTreeMap::new();
    for (name, diode) in diodes {
        values.insert(format!("{name}_G"), diode.g_off);
        values.insert(format!("{name}_Ioff"), 0.0);
    }
    for (name, transient_fn) in &system.transient_sources {
        values.insert(name.clone(), transient_fn.value_at(0.0));
    }
    match system
        .initial_state(&values, general_mna::DEFAULT_INITIAL_STATE_TOLERANCE)
        .map_err(DaeError::InitialCondition)?
    {
        Some(x) => Ok(x),
        None => Ok(vec![0.0; system.order()]),
    }
}

pub fn simulate_transient(
    source: &str,
    dialect: Dialect,
    diodes: &BTreeMap<String, IdealDiode>,
    x_initial: Option<&[f64]>,
    t_final: f64,
    step: TimeStep,
) -> Result<Vec<(f64, OperatingPoint)>, DaeError> {
    let statements = general_mna::parse_and_flatten(source, dialect).map_err(DaeError::Parse)?;
    let system = MnaBuilder::new(dialect)
        .build_statements(&statements)
        .map_err(DaeError::Build)?;

    let mut x_prev = resolve_initial_state(&system, diodes, x_initial)?;
    let mut x_prev_prev: Option<Vec<f64>> = None;
    let mut prev_diode_raw_ioff: BTreeMap<String, f64> = BTreeMap::new();
    let mut prev_segments: Option<Vec<Segment>> = None;
    let mut ringing_cooldown: u32 = 0;

    let mut trace = Vec::new();
    let mut t = 0.0;

    match step {
        TimeStep::Fixed(dt) => {
            let steps = (t_final / dt).round() as usize;
            trace.reserve(steps);
            for step_index in 0..steps {
                t += dt;
                let forced = step_index == 0 || ringing_cooldown > 0;
                let (point, used_backward_euler) = step_with_fallback(
                    &system,
                    &statements,
                    diodes,
                    x_prev_prev.as_deref(),
                    &x_prev,
                    dt,
                    &prev_diode_raw_ioff,
                    &prev_segments,
                    forced,
                    t,
                    &BTreeMap::new(),
                    &BTreeMap::new(),
                )?;
                if used_backward_euler && !forced {
                    ringing_cooldown = RINGING_COOLDOWN_STEPS;
                } else if ringing_cooldown > 0 {
                    ringing_cooldown = ringing_cooldown.saturating_sub(1);
                }

                prev_segments = Some(classify_segments(&point));
                prev_diode_raw_ioff = point
                    .diode_names
                    .iter()
                    .cloned()
                    .zip(point.diode_raw_ioff.iter().copied())
                    .collect();
                x_prev_prev = Some(std::mem::replace(&mut x_prev, point.x.clone()));
                trace.push((t, point));
            }
        }
        TimeStep::Adaptive(config) => {
            let mut dt_next = config.dt_init;
            let mut step_index = 0usize;
            while t < t_final {
                let dt_trial = dt_next.min(t_final - t);
                let forced = step_index == 0 || ringing_cooldown > 0;
                let (point, used_backward_euler, dt_used, next_dt) = step_control::adaptive_step(
                    &system,
                    &statements,
                    diodes,
                    x_prev_prev.as_deref(),
                    &x_prev,
                    dt_trial,
                    &prev_diode_raw_ioff,
                    &prev_segments,
                    forced,
                    &config,
                    t,
                )?;
                if used_backward_euler && !forced {
                    ringing_cooldown = RINGING_COOLDOWN_STEPS;
                } else if ringing_cooldown > 0 {
                    ringing_cooldown = ringing_cooldown.saturating_sub(1);
                }

                t += dt_used;
                dt_next = next_dt;
                prev_segments = Some(classify_segments(&point));
                prev_diode_raw_ioff = point
                    .diode_names
                    .iter()
                    .cloned()
                    .zip(point.diode_raw_ioff.iter().copied())
                    .collect();
                x_prev_prev = Some(std::mem::replace(&mut x_prev, point.x.clone()));
                trace.push((t, point));
                step_index += 1;
            }
        }
    }
    Ok(trace)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Breakdown,
    Leakage,
    Forward,
}

fn classify_segments(point: &OperatingPoint) -> Vec<Segment> {
    point
        .diode_z
        .iter()
        .map(|&(z1, z2)| {
            if z1 > 1e-9 {
                Segment::Breakdown
            } else if z2 > 1e-9 {
                Segment::Forward
            } else {
                Segment::Leakage
            }
        })
        .collect()
}

/// Detects trapezoidal "ringing": trapezoidal integration is A-stable (bounded) but not
/// L-stable, so a very lightly damped mode (e.g. a switch node left nearly floating during
/// ideal switch dead time, with only tiny leakage conductance) doesn't decay under it — it
/// oscillates at the Nyquist frequency (sign-flipping every single step) with roughly
/// constant amplitude instead. Detected as three consecutive values of the same unknown
/// alternating in sign (`x_prev_prev`, `x_prev`, `trial` share the outer two signs, opposite
/// the middle one) with the magnitude not shrinking step to step — the direct signature of a
/// sustained oscillation, as opposed to a legitimate sign change while settling (which
/// shrinks, not sustains, in magnitude) or ordinary numerical noise near zero (excluded by
/// the noise floor). Found and root-caused via `general-simulator`'s LLC validation
/// (`crates/dae-runtime/examples/llc_validation.rs`): the switching node's voltage oscillated
/// between roughly +/-3000V for the entire ~200ns ideal switch dead-time window every period, while
/// every other tracked quantity (notably the actual circuit output) stayed smooth and
/// physically reasonable throughout — falling back to backward Euler (which has no such
/// weakness) the instant this is detected fixes it, consistent with backward Euler already
/// being this crate's answer to every other kind of transient stiffness.
fn is_ringing(x_prev_prev: &[f64], x_prev: &[f64], trial: &[f64]) -> bool {
    const NOISE_FLOOR: f64 = 1e-9;
    x_prev_prev
        .iter()
        .zip(x_prev)
        .zip(trial)
        .any(|((&a, &b), &c)| {
            if a.abs() < NOISE_FLOOR || b.abs() < NOISE_FLOOR || c.abs() < NOISE_FLOOR {
                return false;
            }
            let alternating = (a > 0.0) == (c > 0.0) && (a > 0.0) != (b > 0.0);
            alternating && c.abs() >= b.abs() * 0.9
        })
}

/// How many additional steps to keep using backward Euler after a ringing catch, before
/// trusting trapezoidal again. A single corrective backward-Euler step pulls the ringing
/// mode's *value* back to something reasonable, but doesn't fully re-establish it as "settled"
/// in the sense trapezoidal's own derivation needs (see [`Scheme::Trapezoidal`]'s doc comment
/// on consistent initial conditions) — resuming trapezoidal immediately was observed to let a
/// smaller residual oscillation resume and drift to a stale-but-stable wrong plateau, rather
/// than fully recovering. A short cooldown of plain, unconditionally-stable backward-Euler
/// steps lets the correction actually settle first.
const RINGING_COOLDOWN_STEPS: u32 = 3;

/// Solves one timestep, choosing between [`Scheme::Trapezoidal`] and [`Scheme::BackwardEuler`]
/// the same lagging way [`simulate_transient`] and [`simulate_transient_with_ideal_switches`] both
/// need: `force_backward_euler` covers reasons known *before* solving (the very first step, a
/// gate state that just changed, or an active [`RINGING_COOLDOWN_STEPS`] cooldown); a diode
/// segment change and trapezoidal ringing (see [`is_ringing`]) are only detectable *after*
/// trying trapezoidal, so either one redoes the step with backward Euler. `x_prev_prev` (the
/// state two steps back) is `None` for the first two steps of a run, when ringing can't yet be
/// detected — the existing `force_backward_euler` on the very first step, plus ringing needing
/// at least one full oscillation to show its signature, means this is never a gap in practice.
///
/// Returns `(point, used_backward_euler)` — callers use the flag to drive a
/// [`RINGING_COOLDOWN_STEPS`]-step cooldown after any backward-Euler fallback (not just a
/// ringing-triggered one — a segment or gate change deserves the same settling courtesy).
#[allow(clippy::too_many_arguments)]
fn step_with_fallback(
    system: &MnaSystem,
    statements: &[Statement],
    diodes: &BTreeMap<String, IdealDiode>,
    x_prev_prev: Option<&[f64]>,
    x_prev: &[f64],
    dt: f64,
    prev_diode_raw_ioff: &BTreeMap<String, f64>,
    prev_segments: &Option<Vec<Segment>>,
    force_backward_euler: bool,
    t: f64,
    extra_values: &BTreeMap<String, f64>,
    extra_values_prev: &BTreeMap<String, f64>,
) -> Result<(OperatingPoint, bool), DaeError> {
    if force_backward_euler {
        return fold_and_solve(
            system,
            statements,
            diodes,
            &Scheme::BackwardEuler { x_prev, dt },
            t,
            extra_values,
            extra_values_prev,
        )
        .map(|point| (point, true));
    }
    let trial = fold_and_solve(
        system,
        statements,
        diodes,
        &Scheme::Trapezoidal {
            x_prev,
            dt,
            prev_diode_raw_ioff,
        },
        t,
        extra_values,
        extra_values_prev,
    )?;
    let trial_segments = classify_segments(&trial);
    let segments_changed = Some(&trial_segments) != prev_segments.as_ref();
    let ringing = x_prev_prev.is_some_and(|xpp| is_ringing(xpp, x_prev, &trial.x));
    if segments_changed || ringing {
        fold_and_solve(
            system,
            statements,
            diodes,
            &Scheme::BackwardEuler { x_prev, dt },
            t,
            extra_values,
            extra_values_prev,
        )
        .map(|point| (point, true))
    } else {
        Ok((trial, false))
    }
}

/// Solves the DC operating point of a netlist containing linear devices, ordinary `D` diodes
/// (`diodes`), and any number of `IdealSwitch` instances (`ideal_switches`), each with its own known gate
/// state (see [`GateState`], a re-export of `general_mna::SwitchState` — the same concept: an
/// exogenous, externally-decided mode, not something the LCP resolves).
///
/// A gated-on ideal switch is stamped as a plain `r_on` switch (reusing `general-mna`'s existing
/// switch mechanism — every gated-on ideal switch in one call shares `shared_r_on`, matching that
/// mechanism's own single-shared-resistance design; per-instance `Ron` is a possible future
/// extension, not needed yet). A gated-off ideal switch is folded into the LCP exactly like an
/// ordinary diode, using [`IdealSwitch::body_diode_for_drain_source_stamping`] (not `body_diode`
/// directly) — see that method's own doc comment for why: the netlist declares an ideal switch's two
/// terminals in plain SPICE-conventional `(drain, source)` order, and the mirrored curve is
/// what makes evaluating it against those nodes as-declared give the physically correct
/// result. Each ideal switch element must still use device letter `'D'` in the netlist text (not
/// `'M'`) — see this crate's `docs`/journal for why.
pub fn solve_dc_with_ideal_switches(
    source: &str,
    dialect: Dialect,
    diodes: &BTreeMap<String, IdealDiode>,
    ideal_switches: &BTreeMap<String, (IdealSwitch, GateState)>,
    shared_r_on: f64,
) -> Result<OperatingPoint, DaeError> {
    let statements = general_mna::parse_and_flatten(source, dialect).map_err(DaeError::Parse)?;
    let (system, all_diodes) =
        build_with_ideal_switches(&statements, dialect, diodes, ideal_switches, shared_r_on)?;
    fold_and_solve(
        &system,
        &statements,
        &all_diodes,
        &Scheme::Dc,
        0.0,
        &BTreeMap::new(),
        &BTreeMap::new(),
    )
}

/// Runs a transient simulation of a netlist containing linear devices, ordinary `D` diodes,
/// and any number of `IdealSwitch` instances whose gate state can vary over time — a PWM driver,
/// unlike [`solve_dc_with_ideal_switches`]'s fixed per-call state. `gate_signal(name, t)` is called
/// once per ideal switch per timestep to get that instance's [`GateState`] at time `t`; the caller
/// owns the PWM logic entirely (duty cycle, frequency, phase — this crate has no opinion).
///
/// Because a gate state change means the *symbolic* `general-mna` system itself must be
/// rebuilt (a switch and a `'D'`-stamped diode are structurally different stamps, not just
/// different numeric values — see `docs/architecture.md`), this rebuilds the system every
/// timestep, unlike [`simulate_transient`]'s single build reused throughout. A documented
/// scope choice, not an oversight: fine for the small circuits this crate targets so far: see
/// this crate's journal for the reasoning and what a future optimization would look like.
///
/// Uses the same trapezoidal-with-backward-Euler-fallback policy as [`simulate_transient`],
/// extended to also force backward Euler on any step where *any* ideal switch's gate state just
/// changed from the previous step — a gate transition is exactly the kind of topology change
/// the architecture's "backward Euler after a mode change" policy exists for, arguably more so
/// than a diode's segment change, since it's a real switching event a PWM converter circuit
/// will trigger constantly.
#[allow(clippy::too_many_arguments)]
pub fn simulate_transient_with_ideal_switches(
    source: &str,
    dialect: Dialect,
    diodes: &BTreeMap<String, IdealDiode>,
    ideal_switches: &BTreeMap<String, IdealSwitch>,
    gate_signal: impl Fn(&str, f64) -> GateState,
    shared_r_on: f64,
    x_initial: Option<&[f64]>,
    t_final: f64,
    dt: f64,
) -> Result<Vec<(f64, OperatingPoint)>, DaeError> {
    let states_at = |t: f64| -> BTreeMap<String, (IdealSwitch, GateState)> {
        ideal_switches
            .iter()
            .map(|(name, m)| (name.clone(), (*m, gate_signal(name, t))))
            .collect()
    };

    let statements = general_mna::parse_and_flatten(source, dialect).map_err(DaeError::Parse)?;
    let (system0, _) =
        build_with_ideal_switches(&statements, dialect, diodes, &states_at(0.0), shared_r_on)?;
    let mut x_prev = resolve_initial_state(&system0, diodes, x_initial)?;
    let mut x_prev_prev: Option<Vec<f64>> = None;
    let mut prev_diode_raw_ioff: BTreeMap<String, f64> = BTreeMap::new();
    let mut prev_segments: Option<Vec<Segment>> = None;
    let mut prev_gate_states: Option<BTreeMap<String, GateState>> = None;
    let mut ringing_cooldown: u32 = 0;

    let steps = (t_final / dt).round() as usize;
    let mut trace = Vec::with_capacity(steps);
    let mut t = 0.0;
    for step_index in 0..steps {
        t += dt;
        let states = states_at(t);
        let gate_states: BTreeMap<String, GateState> = states
            .iter()
            .map(|(name, (_, state))| (name.clone(), *state))
            .collect();
        let gate_changed = prev_gate_states.as_ref() != Some(&gate_states);
        let forced = step_index == 0 || gate_changed || ringing_cooldown > 0;

        let (system, all_diodes) =
            build_with_ideal_switches(&statements, dialect, diodes, &states, shared_r_on)?;
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
            &BTreeMap::new(),
            &BTreeMap::new(),
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
        trace.push((t, point));
    }
    Ok(trace)
}

fn build_with_ideal_switches(
    statements: &[Statement],
    dialect: Dialect,
    diodes: &BTreeMap<String, IdealDiode>,
    ideal_switches: &BTreeMap<String, (IdealSwitch, GateState)>,
    shared_r_on: f64,
) -> Result<(MnaSystem, BTreeMap<String, IdealDiode>), DaeError> {
    let mut options = BuildOptions {
        on_resistance: Expression::Constant(shared_r_on),
        ..BuildOptions::default()
    };

    let mut all_diodes = diodes.clone();
    for (name, (switch, state)) in ideal_switches {
        match state {
            GateState::On => options.set_switch(name, GateState::On),
            GateState::Off => {
                all_diodes.insert(name.clone(), switch.body_diode_for_drain_source_stamping());
            }
        }
    }

    let system = MnaBuilder::with_options(dialect, options)
        .build_statements(statements)
        .map_err(DaeError::Build)?;
    Ok((system, all_diodes))
}

#[allow(clippy::too_many_arguments)]
fn fold_and_solve(
    system: &MnaSystem,
    statements: &[Statement],
    diodes: &BTreeMap<String, IdealDiode>,
    scheme: &Scheme,
    t: f64,
    extra_values: &BTreeMap<String, f64>,
    extra_values_prev: &BTreeMap<String, f64>,
) -> Result<OperatingPoint, DaeError> {
    let nodes = topology::diode_nodes(statements);
    let order = system.order();

    // Fix every diode's conductance at its canonical reference slope and its Norton current at
    // zero: this is the "A0"/"u0" the whole LCP fold above is built on. Also supply every
    // time-varying V/I source's own current value at `t` -- `system.transient_sources` is
    // empty for a netlist with no `SIN`/`PULSE`/`EXP`/`PWL`/`SFFM` source, so this is a no-op
    // for every existing (plain, static-source) netlist. `t=0.0` for `Scheme::Dc` matches
    // standard SPICE convention (a DC operating point uses a transient source's own value at
    // time zero, not its DC-only fallback, when no separate `DC` value was given). `extra_values`
    // is this step's full block-graph output map (empty for every caller with no block graph at
    // all) -- `system.evaluate` only ever substitutes a symbol actually present in an
    // expression tree, so passing every block's output here unconditionally is harmless; the
    // *only* symbols that can legitimately appear this way are a `BlockKind::Sig2Phys`
    // converter's own name, since `simulate_transient_with_blocks` already
    // rejects any other block name appearing as a bare V/I source literal before any step runs
    // (see `DaeError::SourceNotSig2PhysicalConverter`).
    let mut base_values = BTreeMap::new();
    for (name, diode) in diodes {
        base_values.insert(format!("{name}_G"), diode.g_off);
        base_values.insert(format!("{name}_Ioff"), 0.0);
    }
    for (name, transient_fn) in &system.transient_sources {
        base_values.insert(name.clone(), transient_fn.value_at(t));
    }
    for (name, value) in extra_values {
        base_values.insert(name.clone(), *value);
    }
    let numeric0 = system.evaluate(&base_values).map_err(DaeError::Evaluate)?;

    // a_eff/u_eff_base: the effective matrix and the part of the effective RHS that doesn't
    // depend on this step's own diode currents (those are added per-diode below, since
    // Trapezoidal's history term needs each diode's own B column). coupling_scale multiplies
    // how much *this* step's resolved diode current contributes to x/V — see Scheme::Trapezoidal.
    let (a_eff, u_eff_base, coupling_scale): (general_mna::Matrix<f64>, Vec<f64>, f64) =
        match scheme {
            Scheme::Dc => (numeric0.a.clone(), numeric0.u.clone(), 1.0),
            Scheme::BackwardEuler { x_prev, dt } => {
                let mut a_eff = numeric0.a.clone();
                for row in 0..order {
                    for col in 0..order {
                        a_eff[(row, col)] += numeric0.k[(row, col)] / dt;
                    }
                }
                let u_eff: Vec<f64> = (0..order)
                    .map(|row| {
                        let k_x: f64 = (0..order)
                            .map(|col| numeric0.k[(row, col)] * x_prev[col])
                            .sum();
                        numeric0.u[row] + k_x / dt
                    })
                    .collect();
                (a_eff, u_eff, 1.0)
            }
            Scheme::Trapezoidal { x_prev, dt, .. } => {
                let mut a_eff = numeric0.a.clone();
                for row in 0..order {
                    for col in 0..order {
                        a_eff[(row, col)] = a_eff[(row, col)] / 2.0 + numeric0.k[(row, col)] / dt;
                    }
                }
                // True trapezoidal RHS uses (B/2)*(u_n + u_{n+1}), not B*u_{n+1} alone (see
                // Scheme::Trapezoidal's own doc comment) -- correct as-is only because every
                // netlist source was constant (u_n == u_{n+1}) until time-varying sources
                // existed. `u_prev` re-evaluates the netlist's own linear sources at `t - dt`
                // for a `TransientFunction` source (an analytic, re-evaluatable function of
                // time), and substitutes `extra_values_prev` (the *previous* step's already-
                // computed block outputs -- a block-driven source has no analytic form to
                // re-evaluate, only the discrete sample already used last step, zero-order-held
                // for that step, exactly the sampled-data convention this whole block graph
                // uses) for a `Sig2Phys`-driven one. Skipped (and `numeric0.u`
                // cloned directly, exactly the previous behavior) only when there are neither,
                // since u_n == u_{n+1} trivially and a second `evaluate` call would be pure
                // overhead.
                let u_prev: Vec<f64> =
                    if system.transient_sources.is_empty() && extra_values_prev.is_empty() {
                        numeric0.u.clone()
                    } else {
                        let mut base_values_prev = base_values.clone();
                        for (name, transient_fn) in &system.transient_sources {
                            base_values_prev.insert(name.clone(), transient_fn.value_at(t - dt));
                        }
                        for (name, value) in extra_values_prev {
                            base_values_prev.insert(name.clone(), *value);
                        }
                        system
                            .evaluate(&base_values_prev)
                            .map_err(DaeError::Evaluate)?
                            .u
                    };
                let u_eff: Vec<f64> = (0..order)
                    .map(|row| {
                        let k_x: f64 = (0..order)
                            .map(|col| numeric0.k[(row, col)] * x_prev[col])
                            .sum();
                        let a_x: f64 = (0..order)
                            .map(|col| numeric0.a[(row, col)] * x_prev[col])
                            .sum();
                        (numeric0.u[row] + u_prev[row]) / 2.0 + k_x / dt - a_x / 2.0
                    })
                    .collect();
                (a_eff, u_eff, 0.5)
            }
        };

    struct DiodeInfo {
        name: String,
        canonical: pwl_devices::IdealDiodeCanonical,
        p: Option<usize>,
        n: Option<usize>,
        w: Vec<f64>,
    }

    let mut u_eff = u_eff_base;
    let mut infos = Vec::with_capacity(diodes.len());
    for (name, diode) in diodes {
        let column = system
            .inputs
            .iter()
            .position(|input| input == name)
            .ok_or_else(|| DaeError::UnknownDiodeInput(name.clone()))?;
        let b_col: Vec<f64> = (0..order).map(|row| numeric0.b[(row, column)]).collect();

        // Trapezoidal history: the previous step's already-known diode current contributes
        // its own (B/2)*raw_Ioff_n term to this step's effective RHS directly (only this
        // step's *new* current needs the LCP-fold coupling_scale treatment below).
        if let Scheme::Trapezoidal {
            prev_diode_raw_ioff,
            ..
        } = scheme
        {
            if let Some(&prev_ioff) = prev_diode_raw_ioff.get(name) {
                for (row, &b) in b_col.iter().enumerate() {
                    u_eff[row] += 0.5 * b * prev_ioff;
                }
            }
        }

        let w = dense_solve(&a_eff, &b_col).map_err(DaeError::Linear)?;

        let terminals = &nodes[name];
        let p = topology::node_index(&system.unknowns, &terminals.positive);
        let n = topology::node_index(&system.unknowns, &terminals.negative);

        infos.push(DiodeInfo {
            name: name.clone(),
            canonical: diode.canonical(),
            p,
            n,
            w,
        });
    }

    let x0 = dense_solve(&a_eff, &u_eff).map_err(DaeError::Linear)?;

    let get = |x: &[f64], index: Option<usize>| index.map(|i| x[i]).unwrap_or(0.0);

    let n_diodes = infos.len();
    let v0: Vec<f64> = infos
        .iter()
        .map(|d| get(&x0, d.p) - get(&x0, d.n))
        .collect();
    // gamma[k][j] = w_j[p_k] - w_j[n_k]: how much diode j's unit raw current moves diode k's
    // own terminal voltage (the diagonal, gamma[k][k], is the self term; everything else is
    // genuine cross-coupling through the shared linear network).
    let gamma: Vec<Vec<f64>> = infos
        .iter()
        .map(|k| {
            infos
                .iter()
                .map(|j| get(&j.w, k.p) - get(&j.w, k.n))
                .collect()
        })
        .collect();

    let dim = 2 * n_diodes;
    let mut m = vec![vec![0.0; dim]; dim];
    let mut q = vec![0.0; dim];
    for k in 0..n_diodes {
        let row_w1 = 2 * k;
        q[row_w1] = v0[k] - infos[k].canonical.v_breakdown;
        for (j, info_j) in infos.iter().enumerate() {
            m[row_w1][2 * j] += -gamma[k][j] * info_j.canonical.delta_br * coupling_scale;
            m[row_w1][2 * j + 1] += gamma[k][j] * info_j.canonical.delta_on * coupling_scale;
        }
        m[row_w1][2 * k] += 1.0;

        let row_w2 = 2 * k + 1;
        q[row_w2] = infos[k].canonical.v_th - v0[k];
        for (j, info_j) in infos.iter().enumerate() {
            m[row_w2][2 * j] += gamma[k][j] * info_j.canonical.delta_br * coupling_scale;
            m[row_w2][2 * j + 1] += -gamma[k][j] * info_j.canonical.delta_on * coupling_scale;
        }
        m[row_w2][2 * k + 1] += 1.0;
    }

    let sol = lcp_solver::solve(&m, &q).map_err(DaeError::Lcp)?;

    let mut x = x0;
    let mut diode_z = Vec::with_capacity(n_diodes);
    let mut diode_raw_ioff = Vec::with_capacity(n_diodes);
    for (k, info) in infos.iter().enumerate() {
        let z1 = sol.z[2 * k];
        let z2 = sol.z[2 * k + 1];
        // True (unscaled) physical current, used for reporting and as the next step's
        // trapezoidal history — NOT the same as how much it moves x this step (that's scaled
        // by coupling_scale, since only half of it entered the effective RHS above).
        let raw_ioff = info.canonical.delta_on * z2 - info.canonical.delta_br * z1;
        let x_contribution = raw_ioff * coupling_scale;
        for (xi, wi) in x.iter_mut().zip(info.w.iter()) {
            *xi += wi * x_contribution;
        }
        diode_z.push((z1, z2));
        diode_raw_ioff.push(raw_ioff);
    }

    Ok(OperatingPoint {
        unknowns: system.unknowns.clone(),
        x,
        diode_names: infos.into_iter().map(|d| d.name).collect(),
        diode_z,
        diode_raw_ioff,
    })
}

#[cfg(test)]
mod scheme_tests {
    //! `Scheme` and `fold_and_solve` are private, so the convergence-rate check that proves
    //! trapezoidal is actually second-order (not just "still passes at the same tolerance")
    //! has to live here rather than in `tests/`.
    use super::*;

    enum Kind {
        BackwardEuler,
        /// Trapezoidal for every step after the first (which is always backward Euler, for a
        /// consistent `x0` — matching `simulate_transient`'s own policy).
        Trapezoidal,
    }

    /// Runs the same RC circuit as `tests/transient.rs`'s
    /// `rc_charging_matches_closed_form_exponential` (`Vc(t) = 5*(1-e^-t)`) with a single
    /// scheme throughout, and returns the error against the closed-form solution at `t_final`.
    fn rc_final_error(kind: &Kind, dt: f64, t_final: f64) -> f64 {
        let source = "V1 a 0 5\nR1 a b 1\nC1 b 0 1";
        let statements = general_mna::parse_and_flatten(source, Dialect::Ngspice).unwrap();
        let system = MnaBuilder::new(Dialect::Ngspice)
            .build_statements(&statements)
            .unwrap();
        let diodes = BTreeMap::new();
        let steps = (t_final / dt).round() as usize;

        let mut x = vec![0.0; system.order()];
        for step in 0..steps {
            let t = (step + 1) as f64 * dt;
            let use_be = matches!(kind, Kind::BackwardEuler) || step == 0;
            let point = if use_be {
                fold_and_solve(
                    &system,
                    &statements,
                    &diodes,
                    &Scheme::BackwardEuler { x_prev: &x, dt },
                    t,
                    &BTreeMap::new(),
                    &BTreeMap::new(),
                )
            } else {
                fold_and_solve(
                    &system,
                    &statements,
                    &diodes,
                    &Scheme::Trapezoidal {
                        x_prev: &x,
                        dt,
                        prev_diode_raw_ioff: &BTreeMap::new(),
                    },
                    t,
                    &BTreeMap::new(),
                    &BTreeMap::new(),
                )
            }
            .unwrap();
            x = point.x;
        }

        let vb_index = system.unknowns.iter().position(|u| u == "V(b)").unwrap();
        let expected = 5.0 * (1.0 - (-t_final).exp());
        (x[vb_index] - expected).abs()
    }

    /// The standard way to verify a method's convergence *order* is to halve `dt` and check
    /// how much the error shrinks, not to compare absolute error magnitudes at one `dt`
    /// against another method's (which depends on unknown, method-specific constants and can
    /// easily mislead). Backward Euler is first-order: halving `dt` should roughly halve the
    /// error. Both checked over the same halving, at deliberately coarse `dt` so the halving
    /// ratio is clearly visible before floating-point/near-exact-cancellation noise matters.
    #[test]
    fn backward_euler_error_roughly_halves_when_dt_halves() {
        let t_final = 2.0;
        let error_coarse = rc_final_error(&Kind::BackwardEuler, 0.1, t_final);
        let error_fine = rc_final_error(&Kind::BackwardEuler, 0.05, t_final);
        let ratio = error_coarse / error_fine;
        assert!(
            (1.7..=2.3).contains(&ratio),
            "backward-Euler error ratio (dt vs dt/2) = {ratio}, expected close to 2.0 (first-order)"
        );
    }

    /// Trapezoidal is second-order: halving `dt` should quarter the error.
    #[test]
    fn trapezoidal_error_roughly_quarters_when_dt_halves() {
        let t_final = 2.0;
        let error_coarse = rc_final_error(&Kind::Trapezoidal, 0.1, t_final);
        let error_fine = rc_final_error(&Kind::Trapezoidal, 0.05, t_final);
        let ratio = error_coarse / error_fine;
        assert!(
            (3.0..=5.5).contains(&ratio),
            "trapezoidal error ratio (dt vs dt/2) = {ratio}, expected close to 4.0 (second-order)"
        );
    }

    /// And, directly: at the same coarse `dt`, trapezoidal's error should be substantially
    /// smaller than backward Euler's (not asserting a specific factor, since that depends on
    /// problem-specific constants — just that it is, robustly, much better).
    #[test]
    fn trapezoidal_is_more_accurate_than_backward_euler_at_the_same_dt() {
        let t_final = 2.0;
        let dt = 0.1;
        let error_be = rc_final_error(&Kind::BackwardEuler, dt, t_final);
        let error_tr = rc_final_error(&Kind::Trapezoidal, dt, t_final);
        assert!(
            error_tr < error_be / 10.0,
            "trapezoidal error {error_tr} should be well under backward-Euler error {error_be} at dt={dt}"
        );
    }
}
