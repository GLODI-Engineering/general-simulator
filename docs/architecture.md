# Architecture

## Why not Newton-Raphson

SPICE-family simulators solve `g(x) = 0` (KCL/KVL plus device equations) via Newton-Raphson:

```text
J(x_k) Δx = -g(x_k)
x_{k+1} = x_k + Δx
```

Device equations like a diode's `I = I_S(e^{V/V_T} - 1)` make this converge poorly unless the
per-iteration change in `V` is clamped — "voltage limiting." Limiting works but breaks the
clean `g(x) = 0` abstraction: `g` and its Jacobian become functions of iteration history, not
just `x`, which makes the technique inconsistent between devices sharing a node and
incompatible with most modern nonlinear-solver enhancements. This is documented in detail,
against the primary Xyce source, in the sibling `internal-archive` repo:
`explanations/xyce/newton-raphson-formulation.md` and
`explanations/xyce/voltage-limiting-current-status.md`.

This project's premise: if every device is piecewise-linear, the circuit is exactly linear
*within* any fixed combination of active segments. So replace "iterate Newton on continuous
device physics" with "resolve, once per timestep, which discrete combination of segments is
active" — a fundamentally different and non-iterative-in-the-Newton-sense numerical problem.

## Mode selection as a Linear Complementarity Problem

For a single PWL diode with three segments (reverse-breakdown-conduction below
`V_breakdown`, near-zero leakage between `V_breakdown` and `V_th`, forward conduction above
`V_th`), define each segment `i` by its own linear companion model `I = G_i V + I_off,i`
and a *guard*: how far the circuit's actual point is from that segment's valid voltage range.
Exactly one segment is "active" (its guard is non-violated) at any operating point for a
well-posed diode curve; encoding "guard slack `w_i` complementary to segment-activity measure
`z_i`, both nonnegative, at most one nonzero" is precisely `w = Mz + q`, `w,z >= 0`, `w.z = 0`
— an LCP. The `M`, `q` for a given circuit topology and set of PWL devices are assembled from
the same companion-model conductances that would otherwise go straight into an MNA stamp.

Solving that LCP (via Lemke's algorithm, `crates/lcp-solver`) picks out one self-consistent
combination of active segments across every PWL device in the circuit simultaneously, in one
finite pivoting pass — no continuous iteration, no clamping, no path-dependence. This is the
same rigorous approach a reference tool uses for its piecewise-linear circuit solver.

The MOSFET's four-way behavior (gate-state × current-sign) needs more than a single
diode-style guard; the exact number of complementarity pairs per MOSFET instance is worked out
concretely in Milestone 4 (see the repository's plan), not assumed here.

## One descriptor system for circuit and continuous blocks alike

`elspice-mna` already produces circuits as the descriptor DAE

```text
A x(t) + K dx(t)/dt = B u(t)
```

Every continuous block this project needs to support (transfer function, state-space,
descriptor state-space, PID, integrator, derivative, and piecewise math blocks like
saturation) is *also* naturally expressible in this exact shape:

- A state-space block `dx/dt = Ax + Bu, y = Cx + Du` is a descriptor system with `K = I`.
- a reference tool's own "Descriptor State-Space" block, `E dx/dt = Ax + Bu`, is textually identical
  to `elspice-mna`'s convention with `K = E` — no translation needed at all.
- A transfer function `N(s)/D(s)` is realized once, via controllable canonical form, into
  `(A, B, C, D)`, then handled exactly like the state-space case.
- Saturation/limiter blocks are piecewise, not smooth — modeled with the same
  segment-plus-guard machinery as PWL devices (`crates/pwl-devices`), not a second mechanism.

So a block-diagram element is just **extra unknowns and extra rows** appended to the circuit's
descriptor system, the same way `elspice-mna` already treats an independent source as an extra
unknown/branch (see `elspice-mna`'s own `docs/architecture.md`, "Unknown ordering"). There is
exactly one global linear(-in-mode) descriptor system assembled per timestep: circuit devices
in their currently-resolved PWL segment, plus every continuous block. LCP mode selection only
ever has to reason about the discrete PWL part; continuous blocks are always linear and never
participate in the complementarity problem.

## Timestep loop (once assembly + LCP formulation exist)

1. Assemble the global `(A, K, B)` for the current timestep: `elspice-mna`'s linear-device
   stamps + each `pwl-devices` element in its currently-assumed segment + every
   `continuous-blocks` fragment.
2. Solve the LCP for the discrete PWL segment combination (`lcp-solver`), warm-started from the
   previous timestep's segments where possible.
3. With segments resolved, do one implicit linear solve (trapezoidal, or backward Euler
   immediately after a mode change) for `x(t+h)`.
4. Advance time; repeat.

No Newton loop, no voltage limiting, anywhere in this sequence.

## Numeric stack

- Rust throughout; `faer` for the sparse/dense linear algebra the circuit-facing crates need.
  `lcp-solver` itself deliberately uses a plain dense tableau (see its own module docs) —
  Lemke's algorithm operates on small, inherently dense systems (one row per complementarity
  pair, i.e. per PWL device), so a sparse representation buys nothing there.
- Implicit trapezoidal integration by default; backward Euler at startup and immediately after
  a resolved mode switch.

## Status

`crates/lcp-solver` (Milestone 1) is implemented and verified against hand-solved fixtures
(`crates/lcp-solver/tests/fixtures.rs`): a trivial no-pivot case, a single-pivot scalar case,
a multi-pivot positive-definite case, a degenerate boundary case, a general
solution-satisfies-its-own-definition sweep, and a genuinely infeasible case that correctly
reports ray termination rather than fabricating an answer.

`crates/pwl-devices` (Milestone 2) implements a 3-segment PWL diode via the Chua-Lin canonical
decomposition described above. Verified two ways: the decomposition matches an independently
written direct piecewise formula at every segment and both breakpoints
(`src/diode.rs` unit tests), and a hand-built LCP for the PCNR paper's two-diode circuit
(`tests/two_diode_circuit.rs`) matches two hand-derived operating points exactly — the first
proof the whole approach reproduces a real circuit's answer with no Newton-Raphson anywhere.
No MOSFET model, no `continuous-blocks`, no `dae-runtime`, no `elspice-pwl-cli` yet — those
are Milestones 3-5, and Milestone 3 has an open cross-repo decision (extend `elspice-mna`'s
public API vs. reimplement minimal linear stamping locally) flagged in the journal rather than
decided unilaterally.
