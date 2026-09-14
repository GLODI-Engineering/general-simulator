# Open extension points

This chapter tracks design questions and known gaps that are genuinely still open — not a
wishlist, and not a duplicate of `design-decisions.md`. An entry here means: the question was
raised, considered against the current architecture, and deliberately left unresolved (either
out of scope, blocked on a decision, or an active bug), rather than silently forgotten. Each
entry says what it would take to close, why it wasn't done now, and — where the code has moved
since the question was first raised — whether it has in fact since been resolved. This project
ships fast enough that a question genuinely open when first written down can be answered a few
weeks later without anyone updating the page that says so; every entry below was re-checked
against the current source and journal, not copied forward from the original skeleton.

## Still open

### A genuinely self-triggering ideal switch

A relay, a fuse, or a real MOSFET operated with no active gate drive (pure natural commutation)
— state-dependent switching that should be resolved *from* circuit state, not commanded
externally. Not foldable into the diode LCP for the same reason the existing ideal-switch
channel state isn't: a switch going from closed to open is a topological change to the fixed
linear system `A0`, not a segment choice at fixed topology (see
`design-decisions.md`'s "Ideal-switch channel state" entry, and
`switch-model-ideal-switch.md`). Confirmed still unimplemented: `pwl_devices::IdealSwitch`'s gate
state remains exogenous (`crates/pwl-devices/src/ideal_switch.rs`), and no relay/fuse-style block
kind exists anywhere in `crates/continuous-blocks` or `general-mna`'s `BlockKind`. Left open
deliberately during the 2026-08-21 robustness Q&A — the user agreed this needs its own
formulation, not a variation on the existing LCP fold, and asked to move on rather than build it
under time pressure (`docs/journal/2026-08.md`, Q5).

**What it would take:** a new complementarity formulation specific to self-triggered switching
(state-dependent guard conditions resolved simultaneously with the rest of the circuit, the way
diode segments are, but for a topological rather than affine change) — a research-and-design
task before any implementation, not a mechanical extension of existing code.

### Per-instance ideal-switch `Ron`

Every ideal switch in one `dae-runtime` call still shares a single `shared_r_on: f64` — confirmed
current: the parameter threads unchanged through `crates/dae-runtime/src/lib.rs`
(`build_with_ideal_switches`, `simulate_transient_with_ideal_switches`, `simulate_closed_loop`)
and `crates/dae-runtime/src/block_graph.rs`'s block-graph transient paths, with no per-instance
override anywhere in the call chain as of this writing. A circuit modeling switches with
genuinely different on-resistances (e.g. a high-side and low-side device from different part
numbers) cannot express that today.

**What it would take:** threading a per-`IdealSwitch`-instance `r_on` from the netlist through to
`build_with_ideal_switches`, replacing the single scalar parameter with a per-instance lookup —
mechanical, but touches every call site listed above plus their test fixtures.

### MIMO `StateSpace`/`TransferFunction` in the block graph

`continuous_blocks::StateSpace` already supports a general `(A, B, C, D)` system internally, but
`crates/dae-runtime/src/block_graph.rs`'s own dispatch still hardcodes single-input/single-output
for both `BlockKind::StateSpace` and `BlockKind::TransferFunction` — confirmed unchanged.
`vector-signals.md`'s own survey (Category 7) explicitly declines to bolt on an elementwise
vector rule for these two block kinds and instead names genuine MIMO support as the correct home
for this: a `Vector(N)` input feeding `B`'s `N` columns, a `Vector(M)` output reading `C`'s `M`
rows, with `x`/RK4 stepping unchanged (already a `Vec<f64>` internally) — recommended as a
coordinated follow-up once the rest of the vector-signal plumbing (the `SignalValue` type,
`evaluate_blocks`'s dispatch) exists, rather than a bespoke rule invented ad hoc.

**What it would take:** exactly what `vector-signals.md` describes — extending
`evaluate_blocks`'s `StateSpace`/`TransferFunction` arms to accept and produce `SignalValue::Vector`,
once vector-signal plumbing is in place for these two kinds specifically (most other block kinds'
vector-signal treatment is already settled per that chapter).

A related, narrower gap in the same neighborhood: `vector-signals.md` (Category 7) also notes
that `Pid`, `Vco`, and `Hysteresis` reject `Vector` inputs outright, since a "vectorized" version
of any of them would mean *N independent instances* (a state-fan-out feature), not an elementwise
op — explicitly flagged there as a possible future extension, distinct from the MIMO question
above, and not attempted.

### PFC Stage 2: q-axis current divergence (actively open, in the sibling repo)

The three-phase active-front-end PFC switching bridge in an internal three-phase PFC
experiment does not yet close its current loop correctly: with
three real bugs already found and fixed (fixed-target duty normalization, mains-scale startup
inrush, and a fixed anti-windup clamp vs. a dynamically achievable range — the last of which drove
this project's own `PidClamp::Dynamic` feature), the commanded-zero `q`-axis current still grows
to tens of amps instead of settling, and the DC bus oscillates instead of tracking its soft-start
ramp. The synchronization chain itself is confirmed correct throughout (`PRK`/`PRK_q` held to full
floating-point precision even while the current loop diverges around it), so the bug is confined
to the current-loop/decoupling/anti-windup interaction under the bridge's real (non-ideal,
delayed) dynamics — see `design-decisions.md`'s "Bench-scale vs. mains-scale" entry for the
related scoping decisions made around this same experiment. Left open rather than forced under
time pressure; not this repo's own bug (the experiment lives in the sibling repo), but recorded
here since it is the concrete, currently-open item behind the ideal-switch/PID machinery this
project does own.

**What it would take:** per the experiment's own README, verify the decoupling/feedforward algebra
(Procedure, Part B) against the real circuit's actual transfer function rather than the idealized
one Stage 1's reduction assumed, since real switching/sampling delay likely breaks an
already-decoupled-plant assumption Stage 1 didn't need to make.

### Intermittent `cscript-ffi` test failures under parallel `cargo test`

Filed as issue #6, not fixed: `cargo test --workspace` at default parallelism fails
intermittently in `cscript-ffi`, a different subset of tests each run, while passing reliably
single-threaded (`docs/journal/2026-09.md`, 2026-09-11 07:45). The fixtures appear to race on
compiling or `dlopen`-ing the same shared object. This matters specifically because
`cargo test --workspace` is a required gate in `AGENTS.md`, and an intermittently-failing
required gate trains a reader to ignore red output instead of investigating it.

**What it would take:** root-causing the actual race (most likely serializing the shared-object
build/load step across `cscript-ffi`'s own test fixtures, or giving each fixture a distinct
build/output path) — not yet investigated beyond confirming the failure is real and
non-deterministic.

## Resolved since originally flagged

Nothing in this chapter's original candidate list has actually been resolved yet — re-checking
each of the four originally-flagged items above against current source (`shared_r_on` still a
single scalar; no relay/fuse block kind exists; `StateSpace`/`TransferFunction` dispatch still
SISO; the PFC Stage 2 experiment's own README still reports the loop unconverged) confirms all
four are still genuinely open, not merely undocumented. This section exists so a future update
has an obvious place to record the first one that does close, rather than silently deleting it
from "Still open" with no trace.

## Forward-looking items folded in from recent design decisions

These are "Revisit if" conditions from `design-decisions.md` that describe a concrete future
trigger rather than a currently active problem — listed here per that chapter's own
cross-reference convention, not duplicated in full:

- **`TimeStep::Adaptive`'s event-prediction clamp** (`earliest_next_event_dt`,
  `crates/dae-runtime/src/block_graph.rs`) only covers the zero-order-hold `sample_time`/
  `PhaseShiftPwm`/`Pwm` shapes today. A future block-graph kind with its own discrete "next
  transition" shape should extend that function's match arms rather than special-casing
  elsewhere. Separately, `kind=octblock` is unconditionally rejected under
  `TimeStep::Adaptive` for unrelated state-rollback reasons; the event-clamp already has a
  (currently unreachable) arm ready for whenever that restriction is lifted.
- **Streaming transient output** (`simulate_transient_with_blocks_streamed`) was applied to the
  block-graph/ideal-switch transient paths, which cover every real netlist in this project that
  could plausibly grow large enough to need it. The no-switch path (`simulate_transient`,
  `lib.rs`) was deliberately not touched and would need the same treatment if a large no-switch
  netlist is ever found to need it.
- **Signal-domain `pwc`/`pwl` naming** would need revisiting if a third interpolation style
  (e.g. cubic/spline breakpoints) is ever needed — the one-keyword-per-style convention doesn't
  scale past two without becoming ambiguous; a parameterized `kind=breakpoints
  interp=constant|linear|cubic` block was already identified as the likely shape for that case.

## Documentation debt adjacent to this list (not a design question, noted for completeness)

Two items surfaced in `docs/journal/2026-09.md` (2026-09-04) are doc/cross-repo gaps rather than
open design questions, and are not duplicated as full entries here since they don't need a design
decision, only follow-through: `general-mna`'s own `BlockKind::Sig2Phys` doc comment still owes
an `## Errors` bullet for the "wired as a circuit node" case (blocked only on this session having
no git access to that sibling checkout, not on any unresolved design question); and the sibling
`gsim-core` GUI tool should ideally refuse to *construct* a document that wires a `sig2phys`
converter as an ordinary node in the first place, rather than relying on this repo's own
downstream `reject_sig2phys_wired_into_circuit` check to catch it after the fact.
