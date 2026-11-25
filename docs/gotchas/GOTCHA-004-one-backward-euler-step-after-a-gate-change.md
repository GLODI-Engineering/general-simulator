---
id: GOTCHA-004
discovered: 2026-11-25
discovered_by: contributor
scope:
  - crates/dae-runtime/src/**
severity: high
status: fixed
reproducibility: full
tags: [integration, trapezoidal, ideal-switch, stiffness]
---

# GOTCHA-004 — One backward-Euler step does not settle a switching node, and the error grows as the step shrinks

## Symptom

A switching node sits tens of volts outside the window its own body diodes clamp it to, for a
few steps after every gate transition, and the excursion gets **larger** as the step size gets
smaller. On a $400\text{V}$ rail whose clamp is $401\text{V}$, a node reached $428\text{V}$;
on an $800\text{V}$ secondary in the report that prompted the fix, $950\text{V}$. Every other
quantity in the same run is smooth, the run is otherwise converged, and nothing errors — so a
voltage-stress figure read off the run is silently wrong by up to $\sim150\text{V}$.

The inverted step dependence is the diagnostic. A trapezoidal overshoot shrinks with the step;
this one does not, which is why the first suspicion was the LCP's segment selection rather than
the integrator.

## Reproduction

`crates/dae-runtime/tests/body_diode_clamp.rs`, which is this reproduction kept as a fixture.
One textbook half-bridge leg is enough: a $400\text{V}$ rail, an `idealswitch` each side, a
capacitor across each switch, an inductor from the switching node to a $200\text{V}$ mid-rail,
and gates that actually switch. Gates *held* off never show it; that is why two small circuits
in the original report clamped correctly while the converter did not.

Which step size shows it depends on the capacitance, so the fixture uses three legs with
$174\text{pF}$, $1.65\text{nF}$ and $16.5\text{nF}$ across each switch. Before the fix, with
`general-simulator` 0.7.0:

| `dt` | leg whose clamp is violated | peak (clamp $401\text{V}$) |
|---|---|---|
| 5 ns | 16.5 nF | 427.9 V (and $-25.9\text{V}$ against a $-1.0\text{V}$ clamp) |
| 0.5 ns | 1.65 nF | 428.1 V (and $-13.8\text{V}$) |
| 0.05 ns | 174 pF | 428.1 V |

After the fix every node stays between $400.2\text{V}$ and $400.6\text{V}$ at all three, and
the same node's peak varies by under $0.1\text{V}$ across them.

## Root cause

The stiff mode of a switching node is the one formed by the conducting device's resistance and
the capacitance on that node,

$$\tau = R_{on} C_{node},$$

which is $7\text{ps}$ for $R_{on} = 0.021\ \Omega$ and $348\text{pF}$ — four orders of
magnitude below any transient step size one would choose. A gate transition excites it with the
full rail voltage.

`dae-runtime`'s policy for an excitation like that is a backward-Euler step followed by a
`RINGING_COOLDOWN_STEPS`-step cooldown of further backward-Euler steps, and
`step_with_fallback`'s doc comment said a gate change got exactly that. The code did not: the
condition was `used_backward_euler && !forced`, and `forced` is precisely "we already knew
before solving that this step needs backward Euler" — the first step of a run and a gate
change. Those two, the only ones that are *mode changes imposed from outside*, were the two the
cooldown skipped. Each got one backward-Euler step and no more. All five transient loops in the
crate had copied the same expression.

One step is not enough, and it is least enough at a small step size. Backward Euler's
amplification factor for the stiff mode is

$$\frac{1}{1 + \Delta t / \tau},$$

so one step removes essentially all of it when $\Delta t \gg \tau$ and proportionally less as
$\Delta t$ approaches $\tau$. Trapezoidal's is

$$\frac{1 - \Delta t / 2\tau}{1 + \Delta t / 2\tau},$$

which in the same regime sits just inside $-1$: whatever the one corrective step left behind
flipped sign every step and barely decayed. So the smaller the step, the more residual was
handed to a scheme that would not damp it — an error that grows as the step shrinks, which is
exactly backwards from how an integration error is supposed to behave and is what made this
look like a segment-selection bug.

`is_ringing`, the backstop that exists for precisely this oscillation, could not catch it
either: it tests the *sign* of the raw unknown, and an oscillation about a $400\text{V}$ rail
never changes sign. It was written against a dead-time node swinging about zero, which does.

## Fix or workaround

`dae_runtime::next_ringing_cooldown` now holds the policy in one place for all five loops, and
restarts the cooldown after every backward-Euler step taken for a fresh reason — including a
gate change and the first step of a run — restarting it only for the cooldown's own steps, which
would never let it end.

Trade-off: a run with no switching in it now takes three backward-Euler steps at the start
instead of one, which moves a result in the fifth significant figure (a $22\ \mu\text{H}$
inductor's current at $10\ \mu\text{s}$ went from $-4.3831\text{A}$ to $-4.3829\text{A}$
against an analytic $-4.3835\text{A}$). That is the cost of starting trapezoidal from a state
that is consistent in the stiff modes rather than only in the slow ones, and it is two orders
of magnitude smaller than the error it removes.

A workaround for an older version: give the switching node real damping. An RC snubber across
each switch raises $\tau$ and damps the mode physically, which is what the LLC validation
example already does and why it does it.

## Prevention

`crates/dae-runtime/tests/body_diode_clamp.rs` asserts, at $5\text{ns}$, $0.5\text{ns}$ and
$0.05\text{ns}$, both that every switching node stays inside the window its body diodes clamp
it to and that the same node's extremes do not depend on the step size. It fails at all three
step sizes without the fix.

The structural lesson is the one the single `next_ringing_cooldown` encodes: a per-step policy
copied into five loops is a policy that will be wrong in five places. If another loop is added,
it calls that function.

## References

- Issue #113 — "A node held by a conducting body diode drifts tens of volts past the clamp, and
  a smaller step makes it last longer".
- `book/dev-guide/src/ringing-and-fallback.md` — the full policy and `is_ringing`'s own limits.
- GOTCHA-002 is the other case where a result was wrong in a way no error reported.

## History

- 2026-11-25: Discovered, root-caused and fixed.
