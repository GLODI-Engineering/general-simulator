# Blocks as a descriptor-DAE fragment

`general-mna` builds circuits as a single descriptor differential-algebraic system,

$$A x(t) + K \dot{x}(t) = B u(t),$$

and every dynamic block this project supports — PID, arbitrary state-space, transfer function,
their discrete-domain counterparts — is deliberately parameterized so it compiles into *exactly*
this same shape rather than into some block-diagram-specific representation of its own. This is
the organizing idea `continuous-blocks`' own module doc comment leads with:

> Every dynamic block compiles to a [`StateSpace`] — the same descriptor-DAE shape `general-mna`
> already uses for circuits (`A x + K dx/dt = B u`, with `K` here called `e` for descriptor
> systems).
>
> — `crates/continuous-blocks/src/lib.rs:11-15`

and `state_space.rs` states it as the type's own reason for existing:

> A descriptor state-space system `E * dx/dt = A*x + B*u`, `y = C*x + D*u` — the same `A x + K
> dx/dt = B u` shape `general-mna` uses for circuits, with `K = E`.
>
> — `crates/continuous-blocks/src/state_space.rs:1-3`

## One shape, three ways to arrive at it

`continuous_blocks::StateSpace` (`crates/continuous-blocks/src/state_space.rs:14-26`) holds five
plain row-major matrices — `a`, `b`, `c`, `d`, and an optional `e` — and every `BlockKind` that
carries continuous dynamics is realized into one of these before it ever runs:

- **Ordinary state-space**, $\dot x = Ax + Bu,\ y = Cx + Du$, is the `e = None` case: `K` is
  implicitly the identity. This is `BlockKind::StateSpace`'s netlist form directly — a netlist
  author who writes `a=`, `b=`, `c=`, `d=` is naming `StateSpace::a/b/c/d` verbatim
  (`general-mna/src/block_graph.rs:624-679`).
- **Descriptor state-space**, $E\dot x = Ax + Bu$, is `e = Some(E)` — textually identical to
  `general-mna`'s own `K` convention, which is exactly why the field is even present on this
  type: nothing in `general-simulator` currently exposes `e=` at the netlist level yet (the
  `StateSpace` block kind's own doc comment records this as a deliberately-unreached path,
  `StateSpaceError::NonInvertibleDescriptorMatrix` "is currently unreachable through this parser"
  — `general-mna/src/block_graph.rs:661-664`), but the type is already shaped for it because the
  underlying math needs no new concept to support it.
- **Transfer function**, a rational $N(s)/D(s)$, is realized *once*, at model-build time, into
  `(A, B, C, D)` via `TransferFunction::to_state_space` (controllable canonical form) — after
  that one conversion it is handled by the identical `StateSpace` machinery as the other two
  cases (`general-mna/src/block_graph.rs:691-693`).

A concrete instance of the first case: a PID controller. `BlockKind::Pid`'s own doc comment gives
the filtered-derivative transfer function

$$C(s) = K_p + \frac{K_i}{s} + \frac{K_d N s}{s + N}$$

(`general-mna/src/block_graph.rs:553-555`) — a pure derivative term is non-causal on its own, so
every real PID filters it, and `continuous_blocks::Pid` carries this compensator as a compiled
`StateSpace` internally. When `evaluate_blocks` steps a `Pid` block it calls
`state_space.rk4_step(x, &error, dt)` on that same `StateSpace` value
(`crates/dae-runtime/src/block_graph.rs:1035`) — the anti-windup logic wrapped around it (two-sided
conditional integration against a `clamp` bound; see `PidClamp`,
`general-mna/src/block_graph.rs:139-159`) is extra bookkeeping on top, but the actual dynamics are
the same $A x + K\dot x = Bu$ fragment as everything else in this family.

A second concrete instance, the genuinely multi-input/multi-output case: `BlockKind::StateSpace`
itself is not constrained to SISO. `StateSpace::inputs()`/`outputs()` read straight off `b`'s
column count and `c`'s row count (`crates/continuous-blocks/src/state_space.rs:63-69`), and
`evaluate_blocks`' own dispatch arm flattens whatever combination of scalar/vector `Signal`s the
netlist declared into one `Vec<f64>` of exactly the expected length before calling `rk4_step`
(`crates/dae-runtime/src/block_graph.rs:1054-1077`) — a `1x1` declaration is simply the
degenerate SISO case of the same code path, not a separately-handled one.

## What is deliberately *not* folded into this shape

Three block kinds are explicit exceptions, and each is an exception for the same underlying
reason: the descriptor-DAE shape is fundamentally *linear* in $x$, and these blocks are not.

- **`Vco`** — its `[0,1)` phase wraparound is a genuine discontinuity; `docs/architecture.md:143-145`
  states this directly ("its `[0,1)` wraparound is a genuine discontinuity a linear system can't
  express"). It carries its own oscillator state and its own `step()`.
- **`Hysteresis`** — a Schmitt-trigger on/off latch is genuine discrete memory (stays on until the
  input drops below `low`, stays off until it rises above `high`), not a continuous state
  variable at all (`docs/architecture.md:149-152`).
- **`Pmsm`** — see `block-graph-rk4.md` for the full treatment; in short, its back-EMF/
  cross-coupling and torque terms are *bilinear* (products of two of its own states), which is
  categorically outside what any $A x + K\dot x = Bu$ fragment, however large, can represent.
  `continuous_blocks::pmsm`'s own module doc comment is explicit about this: "this genuinely
  cannot be expressed as the descriptor system `A x + K dx/dt = B u` every other dynamic block in
  this crate compiles to" (`crates/continuous-blocks/src/pmsm.rs:8-12`).

Piecewise/discontinuous behavior elsewhere in the project — saturation limits, PWL device
segments — uses the same segment-plus-guard machinery as `pwl-devices`, or a bespoke `step()`
like the three blocks above, rather than a second parallel representation
(`docs/architecture.md:64-65`).

## What the unification actually buys today, and what it doesn't yet

It is worth being precise about what "the same shape" currently means in this codebase, because
it is easy to over-read. `docs/architecture.md`'s "One descriptor system for circuit and
continuous blocks alike" section states the eventual architectural goal plainly: "a
block-diagram element is just **extra unknowns and extra rows** appended to the circuit's
descriptor system" (`docs/architecture.md:67`), which — if implemented — would let a controller
and the circuit it drives be solved as one linear(-in-mode) system per timestep, the same way
`general-mna` already treats an independent source as an extra unknown.

That is not what `dae-runtime::block_graph` actually does today, and the module says so in its
own doc comment, without hedging:

> Sampled-data co-simulation: every block reads the *previous* circuit step's measurement, the
> whole graph is evaluated once per circuit step ... and the result decides gate states before
> the circuit step itself is solved.
>
> — `crates/dae-runtime/src/block_graph.rs:27-30`

and `simulate_closed_loop`'s own doc comment is equally direct about this being a deliberate
choice, not a stopgap: the controller "reads the previous step's measured output, steps its own
RK4 integrator, decides this step's gate states ... rather than a fully implicit unified system
— deliberately: this is how a real digital PID+PWM controller actually works, not an
approximation" (`docs/architecture.md:102-106`).

So today's real division of labor is: the descriptor-DAE shape is what makes a block's *own*
internal dynamics representable by exactly the same matrix machinery (`StateSpace::derivative`,
`StateSpace::rk4_step`, the dense Gauss-Jordan solve in
`crates/continuous-blocks/src/state_space.rs:153-195`) that would eventually let it be stamped
directly into `general-mna`'s own system — the same code, the same $(A, K, B)$ convention, no
translation layer between "how a controller's dynamics are expressed" and "how a circuit's
dynamics are expressed." What it does *not* yet mean is that a block's unknowns are literally
rows in the same matrix the circuit's own MNA solve factors each step; that remains the
architectural target `docs/architecture.md` records, wiring "a compiled block's `(A, K, B)` into
`dae-runtime`'s global descriptor system" as an explicitly later milestone
(`crates/continuous-blocks/src/lib.rs:22-24`). The value delivered already is real: every
continuous block, however different its netlist-level parameterization, reduces to the one
shape this whole project already knows how to integrate and reason about — which is what lets a
PID, an arbitrary `(A,B,C,D)` filter, and a hand-derived transfer-function compensator share one
implementation (`StateSpace::rk4_step`) instead of three.
