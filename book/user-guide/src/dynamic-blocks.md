# Dynamic blocks (PID, state-space, transfer function)

Every dynamic block in this chapter carries its own internal state across steps, unlike the
stateless sources and math operations in [Sources and math](sources-and-math.md). The full field
reference for each lives in the [Component Reference](component-reference.md); this chapter is
about which one to reach for and how they relate to each other.

## The unifying idea: a descriptor-DAE fragment, same shape as the circuit

`continuous-blocks`'s own module doc comment states the idea this whole library is built around:
every dynamic block here compiles down to the same descriptor-DAE fragment shape `general-mna`
already uses for the electrical circuit itself:

$$Ax + K\dot{x} = Bu$$

A `kind=statespace` block's own state equations, $\dot{x} = Ax + Bu$, $y = Cx + Du$, are a direct
instance of it; a `kind=pid`'s filtered-derivative
form and a `kind=tf`'s rational $N(s)/D(s)$ are both compiled to the same underlying state-space
form before being stepped. This is why a controller block and a passive circuit element aren't
two different kinds of thing to this project's solver — a `Pid` regulating a converter and the
converter's own inductor current are dynamics of the same mathematical shape, evaluated by
related machinery, not a circuit solver bolted onto an unrelated block-diagram engine.

Practically, this shows up as: a dynamic block is stepped forward *unconditionally* every
timestep via its own RK4 integrator (`StateSpace::rk4_step`, not backward Euler) — a dynamic
block's own state is never part of the circuit's own descriptor-DAE solve, so it doesn't
participate in the same implicit solve a stiff RLC network would need. See the dev guide for the
full numerical detail; this chapter stays at the "what do I declare, and when" level.

## `kind=pid`: fields and anti-windup

A parallel PID with a filtered derivative ($C(s) = K_p + K_i/s + K_d N s/(s+N)$) and two-sided
conditional-integration anti-windup:

```text
NAME kind=pid in=<signal> kp=<f64> ki=<f64> kd=<f64> n=<f64> \
     (clamp_lo=<f64> clamp_hi=<f64> | clamp_lo_in=<signal> clamp_hi_in=<signal>)
```

While the compensator's raw (unclamped) output would exceed its bound, the integral term stops
accumulating in the direction that would make the saturation worse, but keeps integrating
normally in the direction that would relieve it — the controller recovers immediately once the
error's sign allows it to, instead of first having to "unwind" a runaway integral term. `n` is
the derivative filter coefficient (filter pole at `-n`); it must be `> 0` (a non-positive value
puts the pole at the origin or in the right half-plane — a non-causal or outright unstable
filter — and is rejected at parse time).

**Both clamp forms** are legal, and reaching for the right one matters:

- `clamp_lo=`/`clamp_hi=` (**fixed**) — the common case, a constant anti-windup bound.
- `clamp_lo_in=`/`clamp_hi_in=` (**dynamic**) — read fresh from the graph every step, adding two
  extra inputs after the error signal (in that order: error, `clamp_lo_in`, `clamp_hi_in`).

Reach for the dynamic form specifically when the achievable output range is itself
state-dependent. The canonical worked case: a current-loop PID commanding a pole voltage that
can't physically exceed roughly half a DC bus voltage that is itself still rising during a
soft-start ramp. A `Fixed` bound sized for the final steady-state range is badly oversized
early on, so anti-windup never actually engages even though the real plant is already saturated
far below that fixed bound — a sustained, hard-to-diagnose windup-driven oscillation results.
`clamp_lo_in`/`clamp_hi_in` must both be given together or both omitted — giving only one is
rejected at parse time.

## `kind=statespace` vs. `kind=tf`: when to reach for which

Both compile to the same underlying representation and share the same RK4 stepping — the choice
is purely about which form your compensator or filter design is naturally already in:

- **`kind=statespace`** (`a=`/`b=`/`c=`/`d=`) — an arbitrary $(A, B, C, D)$ system, genuinely
  MIMO (`B`'s own column count and `C`'s own row count aren't constrained to 1). Reach for this
  when you have (or derive) state matrices directly — a low-pass filter ahead of a `Pid` to damp
  a resonant plant, say, or any multi-input/multi-output linear block a rational transfer
  function can't represent cleanly.
- **`kind=tf`** (`num=`/`den=`) — a rational $N(s)/D(s)$, SISO by definition, coefficients
  highest-degree first. Reach for this when your design is naturally a transfer function — a
  PID's own realizable filtered-derivative form ($C(s) = K_p + K_i/s + K_d N s/(s+N)$, put over
  one denominator) put directly into `num=`/`den=` instead of `kind=pid`'s `kp`/`ki`/`kd`/`n`
  convenience parameterization, for instance, or any compensator you've already designed in the
  $s$-domain by hand.

Neither has anti-windup — that's specifically a `Pid` output's own concern, not every dynamic
block's. Both reject an improper/singular system at parse time rather than producing a system
that can't be integrated; see the [Component Reference](component-reference.md#state-space)
entries for the exact error text.

## `kind=vco` and `kind=hysteresis`: not state-space, on purpose

Both carry real discontinuities a linear system fundamentally can't express, which is why
neither is built as a `StateSpace` internally:

- **`kind=vco`** — a bare frequency-to-$[0,1)$ ramp oscillator. Its output wraps around
  (discontinuously, by construction) every cycle — a genuine wraparound, not something a linear
  system's continuous state could represent.
- **`kind=hysteresis`** — a Schmitt-trigger comparator (`high=`/`low=` thresholds, output
  `1.0`/`0.0`). Its on/off memory is a genuine discrete latch, used for current-mode control when
  there's no fixed switching frequency to modulate a duty command onto (unlike a `Pid` feeding a
  `kind=pwm`) — a real discontinuity, not a linear dynamic with a very steep slope.

Full rationale for why each needed a dedicated implementation rather than a `StateSpace`
approximation belongs in the dev guide; the short version here is enough to know which one to
reach for.

## The discrete-time counterparts

`kind=discretepid`/`kind=discretestatespace`/`kind=discretetf` are the direct discrete-domain
siblings of `kind=pid`/`kind=statespace`/`kind=tf` — same fields, plus a required `ts=<f64>` /
`freq=<f64>` sample period (unlike `kind=cscript`'s optional one: a discrete system's own
dynamics *are* its sample period, there's no "continuous" fallback, and `ts=variable` isn't
accepted here since a solver-chosen variable schedule doesn't compose with a fixed-period
recursion). `kind=discretepid` additionally takes `integration_method=forward|backward|trapezoidal`
(default `forward`) for how its integral term advances each sample period. See the Component
Reference's own [Discrete PID](component-reference.md#discrete-pid-controller)/[Discrete
State-Space](component-reference.md#discrete-state-space)/[Discrete Transfer
Function](component-reference.md#discrete-transfer-function) entries for the full field
reference, and `book/dev-guide/src/discrete-time-blocks.md` for why the sample-period requirement
is mandatory here specifically.

## Closing a loop through a dynamic block

A controller reading its own previous output, or regulating a block that (directly or
indirectly) feeds back into its own error signal, is a same-step algebraic loop unless you
deliberately route one edge through `prev:` — see [Signals](signals.md) for the full mechanism,
the exact `AlgebraicLoop` error, and when `prev:` is the right tool versus when a same-step cycle
means the netlist is actually wired wrong. The worked examples chapters (Buck/PID, PMSM drive)
show complete cascaded-loop designs built from these blocks with real gain derivations.
