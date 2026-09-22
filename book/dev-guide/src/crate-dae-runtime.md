# `dae-runtime`: circuit assembly and the transient loop

The crate where a netlist becomes numbers: it assembles a circuit's linear MNA system (built
by `general-mna`, including its `'D'` stamp) together with PWL device segments (from
`pwl-devices`) into one Linear Complementarity Problem, solves it (`lcp-solver`), and steps the
result through time. `crates/dae-runtime/src/lib.rs`'s module doc states the invariant this
whole repo exists for: "No Newton-Raphson, no voltage limiting, anywhere in this crate."

## The job boundary, stated and defended

`AGENTS.md`'s "Project boundaries" section assigns this repo exactly two jobs, and everything
in this crate is one of them:

1. **LCP-based mode selection for piecewise-linear devices** — deciding, once per timestep,
   which linear segment of every PWL device is active.
2. **Compiling continuous blocks into descriptor-DAE fragments** of the same
   `A x + K dx/dt = B u` shape circuits use — and, since the format-unification work, also
   *evaluating* the block graph over time (what the blocks *are* and how they parse lives in
   `general-mna`'s `block_graph`; see below).

Everything adjacent is deliberately someone else's: netlist parsing stays in
`general-spice-core` (`dae-runtime` receives an already-parsed, already hierarchy-flattened
statement list — `DaeError::Parse` merely wraps `general_mna::parse_and_flatten`'s failure),
and linear-device MNA stamping (R, C, L, V, I, G, E, F, H) plus numeric Schur-complement
reduction stays in `general-mna` (`MnaBuilder::build_statements` is called once, then the
result is *used* — never re-derived). The boundary is not cosmetic: it is what keeps the
numerically risky part of this project small enough to verify against hand-derived results
(see `verification-discipline.md`).

## Module map

- **`lib.rs`** — the core fold and the diode-only transient loop. The module doc's "generic
  Thevenin/LCP fold" derivation belongs to `lcp-formulation.md`; in one sentence here: each
  diode `k` is stamped as a fixed conductance `{k}_G` (pinned at the canonical reference slope
  `g_off`) plus a per-instance current-source symbol `{k}_Ioff`, which makes the whole linear
  system `A0` genuinely fixed, with every segment's nonlinearity pushed into the `max(0, ...)`
  `z` terms — and since the system is linear in those `Ioff` terms, substituting
  `V_k(z)` into each diode's guard equations *is* the LCP `(M, q)` this crate hands to
  `lcp_solver::solve`. Around that fold: `solve_dc` (diode-only DC operating point),
  `simulate_transient` (diode-only transient; `Scheme::Trapezoidal` most steps,
  `Scheme::BackwardEuler` for the very first step and after any LCP-resolved mode change —
  `dae-integration.md` covers why), `resolve_initial_state` (an explicit `x_initial` always
  wins; otherwise the netlist's own `ic=` values are honored via
  `general_mna::MnaSystem::initial_state`, which *assigns* them rather than solving a
  constrained operating point around them — see the 2026-09-11 journal entry),
  `solve_dc_with_ideal_switches`/`simulate_transient_with_ideal_switches` (gated-on switch
  stamped as a plain `r_on` switch reusing `general-mna`'s switch mechanism; gated-off switch
  folded into the LCP exactly like a diode, via
  `IdealSwitch::body_diode_for_drain_source_stamping` — `crate-pwl-devices.md` has the
  node-order history), plus the pieces every transient path shares: `Segment` classification
  from resolved `(z1, z2)`, `is_ringing` (trapezoidal's Nyquist-frequency oscillation
  signature — three consecutive sign-alternating, non-shrinking values of the same unknown,
  first caught as a ±3000 V oscillation on a nearly-floating switch node in
  `crates/dae-runtime/examples/llc_validation.rs`; full story in `ringing-and-fallback.md`),
  `RINGING_COOLDOWN_STEPS` (three backward-Euler steps after any fallback, because one
  corrective step re-fixed the *value* but not the *settledness* trapezoidal's derivation
  assumes), and `step_with_fallback`, the one function both loops use to pick the scheme —
  reasons known *before* solving (first step, a just-changed gate state, an active cooldown)
  force backward Euler up front; a diode-segment change or ringing, only detectable *after*
  trying trapezoidal, redoes the step with backward Euler.
- **`block_graph.rs`** — the block-graph transient loop (`simulate_transient_with_blocks`,
  plus its `_capped` and `_streamed` variants), and the evaluator for the
  `general_mna::block_graph` types. Its module doc states the ownership split: `general-mna`
  owns *what these types are* and *parsing them out of source text*
  (`general_mna::build_system`); this module owns *evaluating the graph over time*
  (`BlockState`, `evaluate_blocks`, `topological_order`). Also here: gate resolution
  (`resolve_gates` — every `GateBinding` is `Block(name)`, on while the named block's current
  output is `>= 0.5`), the sampled-data co-simulation order (read the previous circuit step's
  measurement → evaluate the whole graph once in the causal order `topological_order` derives
  → resolve gate states → solve the circuit step), `reject_sig2phys_wired_into_circuit` (a
  `sig2phys` converter's name used as a circuit node used to silently read `V(node) = 0`; now
  rejected up front, in both modes, with `DaeError::Sig2PhysUsedAsCircuitNode`), and
  `ADAPTIVE_STEP_HARD_CAP` (10,000,000 accepted adaptive steps — see `DaeError::
  AdaptiveStepStalled`). Causality and cycles are `block-graph-causality.md` /
  `block-graph-cycles.md`; the per-block RK4 convention is `block-graph-rk4.md`.
- **`step_control.rs`** — how a run picks its step size: `TimeStep::Fixed(dt)` (unchanged
  since before this module existed) or `TimeStep::Adaptive(AdaptiveConfig)`,
  local-truncation-error-driven, with `reltol`/`abstol` forming the per-unknown error scale
  `abstol + reltol * max(|x_trap|, |x_be|)` — the module doc notes, rather than hides, the
  simplification of one scalar `abstol` for voltages and currents alike where SPICE has
  `VNTOL`/`ABSTOL`. The user-facing half is `book/user-guide/src/time-stepping.md`.
- **`closed_loop.rs`** — `simulate_closed_loop` and `sawtooth_carrier`: a lighter
  fixed-topology convenience (a `Sum`-then-`Pid`-then-duty-comparator controller wired around
  a circuit's ideal switches, the one case it was first built for), kept for simple direct
  callers. Its module doc is explicit that this is *not* a closed-loop "mode": it is a
  sampled-data co-simulation — the controller reads the circuit's previous-step measurement,
  steps its own dynamics (RK4), and the result decides this step's gates before the circuit
  step solves — "literally how a real digital PID+PWM controller in an actual converter
  works." The general block-graph loop in `block_graph.rs` replaced it for the CLI, which uses
  the general path unconditionally for every ideal-switch-containing transient run.
- **`topology.rs`** — extracts each `D` element's two terminal node names from the caller's
  already-parsed (and per `general_mna::hierarchy::flatten`, already hierarchy-resolved)
  statement list. `general-mna` parses the same information internally but doesn't expose
  per-device topology in its public API; operating on the caller's own `statements` rather
  than re-parsing raw text is what keeps a diode declared inside a `.subckt` body visible
  under its flattened, dotted-path name (`X1.D1`) — a fresh from-scratch text re-parse would
  never see it.
- **`linsolve.rs`** — a small hand-rolled dense solve (Gauss-Jordan, partial pivoting) used to
  fold a circuit's linear part into the LCP. Deliberately hand-rolled rather than `faer`:
  "at this milestone every circuit is small and dense, and a solver this size is easy to read
  and trust outright" — `docs/architecture.md` already commits to `faer` once size or
  sparsity make it worth the dependency.

## Where the rest of the story lives

This chapter is orientation. The depth is elsewhere: `lcp-formulation.md` (the fold into
`(M, q)`), `dae-integration.md` (trapezoidal vs backward Euler), `ringing-and-fallback.md`
(the detection and the cooldown), `switch-model-diodes.md` / `switch-model-ideal-switch.md`
(the endogenous vs exogenous mode split), `block-graph-descriptor.md` /
`block-graph-causality.md` / `block-graph-cycles.md` / `block-graph-rk4.md` (the block graph
itself), `python-blocks.md` / `octave-blocks.md` (the escape-hatch contracts the evaluator
invokes), and `measurements-architecture.md` (why `kind=measure` is deliberately *not* part
of this crate).

## Source material this was adapted from

- `crates/dae-runtime/src/lib.rs` — module doc, `DaeError`, `Scheme`, `solve_dc`,
  `simulate_transient`, `resolve_initial_state`, `Segment`/`classify_segments`, `is_ringing`,
  `RINGING_COOLDOWN_STEPS`, `step_with_fallback`, the ideal-switch entry points.
- `crates/dae-runtime/src/block_graph.rs` — module doc, `reject_sig2phys_wired_into_circuit`,
  `resolve_gates`, the three `simulate_transient_with_blocks*` entry points,
  `ADAPTIVE_STEP_HARD_CAP`.
- `crates/dae-runtime/src/step_control.rs`, `closed_loop.rs`, `topology.rs`, `linsolve.rs` —
  module docs.
- `AGENTS.md` — "Project boundaries".
- `docs/journal/2026-09.md` — the 2026-09-11 `ic=` solve→assignment entry.

## Checkpoint and resume (`checkpoint.rs`)

`simulate_transient_with_blocks_checkpointed` is the streamed transient loop with three
additions, all driven by a `CheckpointControl`: it can *start from* a `Checkpoint`, hand one to
a callback every so much simulated time, and return the final state as one. The plain
`simulate_transient_with_blocks_streamed` is now a thin wrapper passing the default (no
resume, no periodic hand-out, no final snapshot), so no existing caller changed behaviour.

The design rule is that a snapshot is exactly the loop-carried state and nothing else — the
`snapshot` closure inside the loop and `restore_block_states` are the two places to read, and
they are deliberately written as a mirror pair. Two details matter for the bit-identity
guarantee:

- The loop's own `step_index` is saved and restored, so a resumed run's first step is not
  treated as "the first step of a run" (which forces backward Euler). Combined with the saved
  $x_{k-1}$, the resumed step is an ordinary trapezoidal continuation.
- The fixed-step arm computes its step count from `t_final - t`, and `t` itself is restored
  exactly, so the sequence of `t += dt` additions — and hence every rounding — is the one the
  uninterrupted run performed.

`Checkpoint::deck_hash` is a hand-rolled FNV-1a over the `Debug` rendering of the flattened
statements, dialect, blocks, models, gates and shared on-resistance — not `DefaultHasher`,
whose output is not stable across Rust versions and would make a file from one toolchain refuse
under another. A hash mismatch or an `unknowns` mismatch is `DaeError::CheckpointDeckMismatch`.

The final snapshot is opt-in (`CheckpointControl::final_snapshot`) rather than unconditional
for one reason: a `cscript` whose library does not export the checkpoint state contract cannot
be snapshotted, and that must only surface when a checkpoint was actually asked for. The first
cut of this feature returned a final snapshot always and broke every `cscript` CLI test — a
good illustration of why the refusal is lazy, not up front.

The escape hatches each carry their opaque state as bytes in `BlockSnapshot`:

- `pyblock` — `pickle`, no opt-in.
- `octblock` — `OctaveSession::save_state`/`load_state`: the `__gs_state.<name>` slot is copied
  to a scratch variable, `save('-binary', ...)`d to a temporary file under `std::env::temp_dir`
  (unique per process, child and call), read into Rust and deleted; `load` is the mirror. It
  rides the same marker-framed call protocol as every other session method, so an Octave-side
  failure is an ordinary `OctaveError::Runtime` and the session survives. No opt-in.
- `cscript` — `CScriptInstance::state_bytes`/`restore_state` over the
  `cscript_state_size`/`_write`/`_read` triple, resolved in `CScriptLibrary::load` as one
  `Option<StateIo>` so a partial set cannot exist past load time (it is a `MissingSymbol`
  naming the absent one). `supports_state_io()` is the up-front check the snapshot arm makes
  before refusing with `CheckpointUnsupportedBlock`. The restore arm re-checks it, because the
  deck hash covers the `lib=` path, not the `.so`'s contents.

The `CScript`/`OctBlock` variants were appended to `BlockSnapshot` after `PyBlock`; postcard
encodes a variant as its index, so every existing file decodes unchanged and `FORMAT_VERSION`
stayed at 1.
