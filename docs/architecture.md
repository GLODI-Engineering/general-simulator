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
- A standard "Descriptor State-Space" block, `E dx/dt = Ax + Bu`, is textually identical
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

## Timestep loop

`dae_runtime::simulate_transient` (Milestone 6) implements this for diode-only circuits today:

1. Assemble the effective backward-Euler matrix/RHS once (`A + K/dt`, independent of `x_prev`
   — only the RHS's `(K/dt) x_prev` term changes per step) via the same reference-conductance
   fold Milestones 2-3 already use.
2. Each step: fold the diode LCP against that effective matrix/RHS (identical code path to
   `solve_dc`, since it only ever depended on "the effective matrix/RHS" being fixed for a
   given solve — not on what they are), solve it (`lcp-solver`), recover `x(t+h)`.
3. Advance time; repeat.

No Newton loop, no voltage limiting, anywhere in this sequence. Full trapezoidal integration
(second-order, matching Xyce/SPICE's own default) is now implemented alongside backward Euler,
selected automatically: backward Euler for the mandatory first step and any step whose resolved
diode segments differ from the previous step's (trapezoidal's derivation needs `x_n` to already
satisfy the circuit's algebraic constraints exactly — the DAE analog of "needs consistent
initial conditions" — which only a just-completed backward-Euler-or-DC step guarantees),
trapezoidal otherwise. Verified by convergence order (halving `dt`, not comparing one-`dt`
error magnitudes): backward Euler roughly halves error, trapezoidal roughly quarters it.

`simulate_transient_with_mosfets` extends this to MOSFETs with a caller-supplied time-varying
gate signal (PWM), rebuilding the symbolic `elspice-mna` system every step (a gate transition
is a structural stamp change, not just a numeric one) and forcing backward Euler on any step
whose gate states differ from the previous step's, on top of the existing diode-segment-change
fallback.

`dae_runtime::simulate_closed_loop` wires a `continuous-blocks` controller (a compiled `Pid`)
into a real closed loop around a circuit's MOSFET gate(s), as a sampled-data co-simulation
(controller reads the previous step's measured output, steps its own RK4 integrator, decides
this step's gate states via a caller-supplied PWM comparator) rather than a fully implicit
unified system — deliberately: this is how a real digital PID+PWM controller actually works,
not an approximation. It also applies **two-sided conditional-integration anti-windup**
against a caller-supplied output range: if a tentative controller step would push the output
further past either saturation rail while the current error still drives it that way, the
controller's state is frozen rather than advanced. This directly fixes a documented real
failure mode: a Xyce/ngspice closed-loop boost-PI experiment in the sibling
`internal-archive` repo (`experiments/converters-benchmark-boost-pid`,
`gotchas/xyce-boost-pi-nonlinear-failure-integrator-windup.md`) used one-sided anti-windup and,
per that experiment's own conclusion, never actually achieved working regulation — the
integrator wound down past recovery during startup overshoot, PWM floored at zero, and the
converter stopped switching for the rest of the run, with the misleadingly-plausible final
voltage reading being nothing more than the output capacitor discharging through the load.
`elspice-pwl`'s two-sided version, at correctly-scaled gains, achieves real sustained
regulation on the identical circuit spec — see `crates/dae-runtime/tests/closed_loop_boost_anti_windup.rs`.

`simulate_closed_loop` bakes in one fixed topology (a single `Pid` feeding a duty-modulated PWM
comparator) — the right shape for the common buck/boost case, but not general. `dae_runtime::
block_graph::simulate_transient_with_blocks` generalizes this: gates are resolved from a graph
of named, independently reusable `continuous-blocks` blocks (`Const`, `Pwl`, `Sum`, `Gain`,
`Pid`, `StateSpace`, `TransferFunction`, `Vco`) wired together by the caller, evaluated once
per circuit step in declaration order — the same discipline a real block-diagram tool
(a reference tool, a reference tool) uses. An error signal is a `Sum` block's own output (explicit `+`/`-` signs),
not something a controller computes internally; a frequency-modulated PWM carrier (needed for
LLC-family converters, which regulate by switching *frequency* rather than duty, unlike
buck/boost) is `Sum -> Pid -> Sum -> Vco`, four separately testable blocks, not one fused
function; a PID with a filtered derivative can be given directly as a `TransferFunction`'s own
`N(s)/D(s)` coefficients instead of `Pid`'s `Kp`/`Ki`/`Kd` convenience constructor, and an
arbitrary compensator/filter as a `StateSpace`'s own `(A, B, C, D)` matrices. `Vco` is
deliberately not a `StateSpace` itself: its `[0,1)` wraparound is a genuine discontinuity a
linear system can't express, the same reason `math_ops::saturation` is evaluated directly
rather than folded into one; a separate stateless `math_ops::pwm_from_ramp` compares a shared
oscillator's ramp against a per-gate phase/duty, so one `Vco` can drive several independently-
phased gates (a half-bridge's two complementary switches) without needing one oscillator
instance per gate.

Crucially, `GateBinding` covers *every* gate kind — `Fixed`, `PwmFixed` (fixed-frequency/fixed-
duty, no block graph needed), `Vco`, and `Pwm` (block-driven duty) — resolved by exactly the
same per-step loop. There is deliberately no separate "closed-loop" function or CLI mode: a
real circuit simulator has no such mode either (a transient analysis is a transient analysis;
whether a gate's block chain happens to read the circuit's own state back via
`Signal::Measure` is a property of how the netlist is wired, not something the tool needs
telling in advance) — `elspice-pwl-cli`'s ordinary `--mode transient` resolves both the
historically "open-loop" and "closed-loop" cases through this one function. See that crate's
own module doc comment for the full device-file grammar and an LLC-converter example, and
`internal-archive/experiments/elspice-pwl-llc-closed-loop-vs-xyce-ngspice/` for the
worked comparison this was built for.

`crates/elspice-pwl-cli` (binary `elspice-pwl`) is a thin netlist-in/CSV-waveform-out runner
over `dae-runtime`'s public API, with a small hand-rolled device-params file format (no
serde/TOML dependency for something this simple). Tested by spawning the actual built binary
against fixture netlists that are the exact circuits already hand-verified elsewhere in this
workspace — a cross-check, not a fresh derivation.

All six original milestones, plus everything explicitly deferred from them, are now
implemented. See the journal for what remains genuinely open (validation against Xyce/ngspice
baselines, full converter benchmarks, per-instance MOSFET `Ron`, and the smaller scope notes
scattered through each milestone's own entry).

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

`crates/dae-runtime` (Milestone 3) closes the loop for diodes: `elspice-mna` was extended
(sibling repo, commit `9d190db`) to stamp `'D'` elements as a fixed symbolic conductance plus a
Norton current source, and `dae-runtime` folds any netlist's linear part plus any number of
diodes into one LCP via a generic Thevenin-style reduction (fix every diode's conductance at
its canonical reference slope, solve once for a baseline operating point and once per diode for
a sensitivity vector, express every diode's voltage as an affine function of every diode's `z`
variables including cross-coupling, substitute into the LCP). Verified by reproducing
Milestone 2's hand-derived two-diode numbers through real netlist parsing instead of a
hand-typed `(M, q)` — see `crates/dae-runtime/tests/two_diode_via_netlist.rs`.

`pwl_devices::Mosfet` (Milestone 4) needed **no further `elspice-mna` changes**: a gated-on
MOSFET is a plain bidirectional resistor (reuses `elspice-mna`'s existing switch mechanism), a
gated-off MOSFET falls back to its intrinsic body diode (reuses the `'D'` stamp and LCP fold
unchanged, via `Mosfet::body_diode: Diode`). Gate state is exogenous — decided by the caller
per timestep, not resolved by the LCP — so `dae-runtime::solve_dc_with_mosfets` just routes
each instance to whichever mechanism its current gate state calls for. Getting the body
diode's polarity right reuses `Diode` completely unchanged via a node-order convention
(`(source, drain)`, not the datasheet `(drain, source)`) — see `Mosfet`'s doc comment.
MOSFETs must use device letter `'D'` in netlist text (not `'M'`, which `spice-core` correctly
enforces real 4-node SPICE grammar for).

`crates/continuous-blocks` (Milestone 5) implements the transfer-function/state-space/PID/
integrator/math-op compilation described above, standalone and verified against hand-derived
results before any circuit wiring: a first-order lowpass and a pure integrator's step
responses checked against closed-form solutions via its own RK4 stepper, and — the strongest
check — a PID with `Kd=0` where the derivative-filter pole cancels *exactly* against a
numerator zero, so its full (non-minimal) 2-state realization's step response equals the ideal
`y(t) = Kp + Ki*t` for every `t`, not just asymptotically. Wiring a compiled block's `(A, K, B)`
into `dae-runtime`'s global system is still open — meaningful only once transient integration
(Milestone 6) exists, since a controller has nothing to do at a single DC operating point.

`dae_runtime::simulate_transient` (Milestone 6) closes the DC-only gap for diode circuits, with
both backward Euler and full trapezoidal timestepping (trapezoidal by default, falling back to
backward Euler after the mandatory first step and after any LCP-resolved mode change) —
verified against an algebraic circuit's exact DC answer at every step, a plain RC charge curve,
an RC-through-a-diode circuit against a hand-derived closed-form solution combining both
mechanisms, and directly by convergence order (backward Euler ~halves error as `dt` halves,
trapezoidal ~quarters it). `simulate_transient_with_mosfets`, `simulate_closed_loop`, and
`elspice-pwl-cli` (all described in "Timestep loop" above) closed every gap this paragraph
used to note as still open — see the journal for the full account, including the two-sided
anti-windup fix and the cross-simulator validation against Xyce/ngspice on buck, boost
(open- and closed-loop), and LLC — rather than repeating it here.
behind each of those scope choices. No `elspice-pwl-cli` yet.
