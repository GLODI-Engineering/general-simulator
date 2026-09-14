# FAQ

Answers grounded in things actually asked and answered — the robustness Q&A recorded in
`docs/journal/2026-08.md` (2026-08-21, six pointed questions from the user ahead of the
documentation push), the design rationale in `docs/architecture.md`, and the questions the
worked examples surfaced. This page deliberately grows from real questions; entries no one
has asked don't appear here.

## Why not just use ngspice/Xyce?

They solve nonlinear device equations with Newton-Raphson, which needs voltage limiting to
converge on exponential diode/transistor curves — a known wart (path-dependent, inconsistent
between devices sharing a node) even by its own maintainers' account. This simulator
sidesteps it: every active device is piecewise-linear, so a fixed segment combination is
exactly linear, and the per-timestep question "which segments are active" is a discrete LCP
solve (Lemke's algorithm) instead of continuous Newton iteration. The full argument, against
the primary Xyce source, is in the dev guide's
[Why not Newton-Raphson](../dev-guide/architecture-overview.md) chapter and in an internal
set of write-ups analyzing Xyce's own formulation documents.

## Why is it called "ideal switch" and not "MOSFET"?

Because this crate models a PWL companion model — `r_on` channel when gated on, body diode
when gated off — not BSIM-style transistor physics. The name "MOSFET" is reserved for a
future, not-yet-implemented real-transistor model; the rename and its full reasoning are in
`crates/pwl-devices/src/ideal_switch.rs`'s doc comment and `docs/journal/2026-08.md`
(2026-08-29). See [PWL devices](pwl-devices.md).

## Can I model a switch that turns on/off based on its own circuit state (not gate-commanded)?

Not today. A relay, fuse, or a MOSFET with no active gate drive (pure natural commutation)
would need state-dependent switching resolved *from* circuit state — and a closed-to-open
transition is a topological change to the fixed linear system, not a segment choice, so it
doesn't fold into the diode LCP the way everything else here does. The question was asked,
considered, and deliberately left open (with what it would take to close) — see the dev
guide's [Open extension points](../dev-guide/open-questions.md), "A genuinely
self-triggering ideal switch."

## Why do I need `prev:` here instead of just wiring it directly?

You only need it when the wiring forms a same-step cycle — e.g. a current controller
regulating a `pmsm` block's own `id`/`iq` outputs, or a PLL's angle estimate feeding the very
`park` block that produced its error signal. A direct reference there is a genuine algebraic
loop, rejected before any step runs with the exact closing path
(`AlgebraicLoop { cycle: ["A", "B", "A"] }`); `prev:<block>` is the sanctioned one-sample
delay that turns it into a legitimate sampled-data feedback path — the same delay a real
digital controller reading its own last output already has. Ordering itself is automatic;
`prev:` has nothing to do with declaration order. See
[Signals](signals.md), "When to reach for `prev:`."

## My duty/gate command is way outside [0,1] and the loop misbehaves — what's happening?

The classic symptom of a PID whose anti-windup never engages: a fixed `clamp_lo=`/`clamp_hi=`
bound sized for the final steady-state range is badly oversized while the circuit is still
ramping up, so the integrator winds up even though the real plant is already saturated — the
worked case is a current-loop PID commanding a pole voltage that can't exceed roughly half a
DC bus voltage that is itself still rising during a soft-start ramp (sustained oscillation,
output pinned at the clamp, never recovering). The fix is the dynamic clamp:
`clamp_lo_in=`/`clamp_hi_in=`, which reads the bound fresh from the graph every step. The
full debugging account that produced that feature is in an internal three-phase PFC
experiment's write-up
("Fixed PID anti-windup clamp vs. a dynamic achievable range" — the deliberate one-step-ahead
prediction that root-caused it); the user-facing summary is
[Dynamic blocks](dynamic-blocks.md) and the [gotchas](gotchas.md) entry.

## How is the block execution order decided — do I have to declare blocks in causal order?

No. Each step's evaluation order is a genuine topological sort of the `in=`/`inputs=`
dependency graph, computed once per run before any step solves — a block may reference any
other block declared anywhere in the file, before or after it. This was not always true
(declaration order was once a manual discipline, per the robustness Q&A recorded in
`docs/journal/2026-08.md`, Q1 — fixed the same session it was asked). What *is* still
rejected is a genuine same-step cycle, as the `prev:` question above explains. See
[Signals](signals.md), "Ordering is automatic."

## How does the solver decide when to re-check device modes?

Two different mechanisms, deliberately: ideal-switch gate state is never "monitored" — it is
an exogenous command decided entirely outside the DAE/LCP machinery (Q4 of the recorded
robustness Q&A), while diode/body-diode segment selection is genuinely endogenous: every
diode contributes complementarity pairs and *all* of them are stacked into one `(M, q)` LCP
and solved in a single Lemke pivoting pass, simultaneously across the whole circuit. The
per-step mechanics (what forces backward Euler, when a trapezoidal trial is redone) are the
dev guide's [Ringings and fallback](../dev-guide/ringing-and-fallback.md) and
[DAE integration](../dev-guide/dae-integration.md) chapters.

## Source material this was adapted from

- `docs/journal/2026-08.md` — the 2026-08-21 robustness Q&A (Q1–Q4) and the 2026-08-29 rename
  entries.
- `README.md` — "Why", "Status".
- `crates/pwl-devices/src/ideal_switch.rs` — the naming rationale.
- `book/dev-guide/src/architecture-overview.md`, `open-questions.md` — linked answers.
- `book/user-guide/src/signals.md`, `dynamic-blocks.md`, `gotchas.md` — linked answers.
- An internal three-phase PFC experiment's write-up — the anti-windup root-cause account.
