# Discrete-time blocks: state-space, transfer function, PID

*(Design draft, not yet implemented — the actual code change follows once the open questions
below are settled. Scoped to the three block kinds actually being built this pass; a standalone
discrete integrator, external-edge reset variants, wrapping, and back-calculation anti-windup
are candidates for a later pass, not designed here — see "Deferred.")*

## Why these are architecturally simpler than they might look

The existing `kind=statespace`/`kind=tf`/`kind=pid` are all **continuous**: `dae-runtime` RK4-
integrates their `(A,B,C,D)` realization every circuit step, using `dt`. A discrete block is
*not* a continuous block sampled periodically — its own `A`/`B`/`C`/`D` (or transfer-function
coefficients) are already discrete-domain, given directly by the netlist author, no continuous-
to-discrete conversion happens anywhere in this codebase. `x[i+1] = A*x[i] + B*u[i]` is a plain
linear recursion, evaluated once per declared sample period — no RK4, no `dt`-dependent
integration at all. This means:

- **`StateSpace`'s own struct is reused verbatim** — `a=`/`b=`/`c=`/`d=` netlist fields, the
  exact same shape `kind=statespace` already parses. Only one new method is needed:
  `StateSpace::discrete_step(x, u) -> Vec<f64>` (`A*x + B*u`, no `dt`), alongside the already-
  existing `StateSpace::output` (`C*x + D*u`, unchanged — the output equation is identical in
  both domains).
- **`TransferFunction::to_state_space()` is reused verbatim too.** Converting `N(z)/D(z)`
  coefficients to a companion-form realization is purely coefficient algebra — it doesn't know
  or care whether the variable is called `s` or `z`. The existing continuous conversion function
  already does exactly the right thing for a discrete transfer function; nothing new to write.
- **Sample time is mandatory, always `Periodic`, never `None`/`Variable`.** A discrete system's
  own dynamics *are* its sample period — there's no "continuous" fallback the way `cscript`'s
  `sample_time=None` means "every step." `general-mna`'s existing `SampleTimeSpec::Periodic`
  (already built for `cscript`/`pyblock`/`pyfunc`'s own `ts=`/`freq=`/`to=`) is reused directly,
  just required rather than optional for these three kinds. `Variable` is rejected at parse time
  — a discrete system's own recursion is defined *by* a fixed period; "the block decides its own
  next execution time" doesn't compose with that at all.
- **Zero-order hold between sample hits reuses the exact accumulator `cscript`/`pyblock` already
  have** — `dae-runtime`'s existing `time_since_sample`-vs-`period` due-gating, unchanged,
  driving `discrete_step` instead of `rk4_step` when due.

## A real docling finding this design depended on, not assumed

While reading the source manual's own "Discrete Integrator" section to derive the discrete PID's
internal derivative-filter recursion, the Forward Euler/Backward Euler formulas came out of the
document's own automated conversion **swapped between the two headings** — individually well-
formed, wrong association. Caught by cross-checking against the same page's own "only Backward
Euler/Trapezoidal have direct feedthrough" note (inconsistent with the swapped assignment), then
confirmed directly against the source PDF. See `internal-archive`'s own
`gotchas/docling-reading-order-swaps-adjacent-subsections.md` for the full account. The formulas
below are the corrected, PDF-verified versions — and the correction is independently
cross-checked twice more below (the derivative filter's own "always Forward Euler" requirement,
and the `q(z)` formulas in the PID's own combined transfer function, both only make sense with
the corrected assignment).

## Category 1 — Discrete State-Space (`kind=discretestatespace`)

`x[i+1] = A*x[i] + B*u[i]`, `y[i] = C*x[i] + D*u[i]` — same `a=`/`b=`/`c=`/`d=` fields as
`kind=statespace`, same MIMO shape (already genuinely multi-input/multi-output, per
`vector-signals.md`'s own closed decision), plus a required `ts=`/`freq=`/`to=`. `BlockState`
reuses the same `{ state_space, x }` shape the continuous `Dynamic` variant already has —
`dae-runtime`'s own due-gating decides *when* `discrete_step` runs; the state itself needs no
new shape.

## Category 2 — Discrete Transfer Function (`kind=discretetf`)

`Y(z)/U(z) = (n_n z^n + ... + n_0) / (d_n z^n + ... + d_0)`, coefficients highest-degree first —
same `num=`/`den=` fields as `kind=tf`, realized via the existing
`TransferFunction::to_state_space()` (unchanged), evaluated via `discrete_step` the same way
Category 1 is. Genuinely SISO, same as the continuous version (no matrix generalization for a
rational transfer function).

## Category 3 — Discrete PID (`kind=discretepid`)

Reuses `kp=`/`ki=`/`kd=`/`n=` (the derivative filter coefficient — kept as `n`, this codebase's
own existing name for the continuous PID's identical concept, rather than importing the source
manual's own separate `Kf` spelling for what is the same idea) and the existing `PidClamp`
(`clamp_lo=`/`clamp_hi=` fixed, or `clamp_lo_in=`/`clamp_hi_in=` dynamic) — same anti-windup
mechanism the continuous PID already has, unchanged. What's genuinely new is *how* the integral
and derivative actions advance, once per declared sample period instead of continuously:

**Not built by converting to one combined z-domain transfer function and back to state-space**
(the way the *continuous* `Pid::to_transfer_function()`/`to_state_space()` chain works) — that
would need real symbolic polynomial algebra (combining `Kp + Ki*q(z) + <derivative term>` over a
common denominator) that's easy to get subtly wrong by hand and hard to verify by inspection.
Instead, **each of the three actions is realized directly from the block diagram** (source
manual's own Fig. 16.34: three parallel branches, summed), each branch's own recursion already
independently hand-derivable and cross-checkable against the corrected Discrete Integrator
formulas above:

- **Proportional**: `Kp * e[k]` — stateless, no persisted value.
- **Integral**: `Ki * y_i[k]`, where `y_i` is a plain discrete integrator on the raw error `e`,
  using whichever of the three methods (`integration_method=forward|backward|trapezoidal`) the
  netlist declares — the *exact* corrected formulas from "Discrete Integrator" above, applied to
  `e` directly (not filtered).
- **Derivative**: `Kd * v[k]`, where `v[k] = n * (Kd_error - filt_state[k])`... — see the actual
  recursion below. The internal low-pass filter is realized as **Forward Euler specifically,
  always**, regardless of what the netlist's own `integration_method=` picks for the integral
  action — matching the source manual's own explicit statement, and *not arbitrary*: Forward
  Euler is the only one of the three methods with no direct feedthrough (confirmed by the
  corrected formulas above), and this filter's own topology feeds its integrator's output back
  into the very sum that computes the integrator's next input — Backward Euler or Trapezoidal
  here would create an unresolvable same-step algebraic loop. This is the *second* independent
  cross-check confirming the docling correction: the source manual's own design constraint
  ("always Forward Euler" for this specific filter) only makes structural sense under the
  corrected (no-feedthrough) formula, not the swapped one.

**Two persisted state values per branch that needs them** (not a generic `(A,B,C,D)`
realization at all — this block gets its own dedicated `BlockState` shape, not a reuse of
Category 1/2's `{ state_space, x }`):

```text
Integral branch (Forward Euler, e.g.):        int_state[k], next = int_state[k-1] + T*e[k-1]
                                               (needs int_state AND a remembered e[k-1])
Integral branch (Backward Euler):             int_state[k] = int_state[k-1] + T*e[k]
                                               (feedthrough -- no extra remembered-input state)
Derivative filter (always Forward Euler):     filt_state[k] = filt_state[k-1] + T*v[k-1]
                                               v[k] = n * (Kd*e[k] - filt_state[k])
                                               (needs filt_state AND a remembered v[k-1])
```

`u[k] = Kp*e[k] + Ki*int_state_or_output[k] + v[k]`, then the *same* anti-windup clamp logic the
continuous `Pid`'s own `dae-runtime` arm already uses (tentative step, check whether the clamp
would engage further, reject/accept) — reused directly, not reinvented, since the clamp decision
doesn't care whether the underlying step was RK4 or this discrete recursion.

## Foundational changes this touches, regardless of category

- `StateSpace::discrete_step` (new method, `continuous-blocks`).
- Three new `BlockKind` variants (`general-mna`): `DiscreteStateSpace(StateSpace)`,
  `DiscreteTransferFunction(TransferFunction)`, `DiscretePid { pid: DiscretePid, clamp:
  PidClamp }` — each requiring `sample_time: SampleTimeSpec::Periodic` (parse-time error if
  `ts=`/`freq=` is missing or `ts=variable` is given).
- New `continuous_blocks::discrete_pid` module: `DiscreteIntegrationMethod`, `DiscretePid`
  (`kp`/`ki`/`kd`/`n`/`period`/`method`), `DiscretePidState`, a pure `step(&mut self, e: f64,
  state: &mut DiscretePidState) -> f64` — independent of `dae-runtime`, matching the existing
  "math lives in `continuous-blocks`, evaluation lives in `dae-runtime`" split.
- `dae-runtime`: `BlockState::DiscreteDynamic { state_space, x, time_since_sample }` (Categories
  1/2, reusing the existing zero-order-hold accumulator), `BlockState::DiscretePid { state:
  DiscretePidState, time_since_sample }` (Category 3).
- **Not affected**: `SignalValue`, `topological_order`, the physical/signal-domain converter
  boundary. The continuous `StateSpace`/`TransferFunction`/`Pid` block kinds and their own
  `dae-runtime` arms are completely untouched — these are new, additive sibling kinds, the same
  "genuinely separate contract, not a mode of the existing one" precedent `kind=pyfunc` set
  relative to `kind=pyblock`.

## Deferred (explicitly out of scope for this pass)

- A standalone `kind=discreteintegrator` — the integral/derivative recursions above are built as
  private implementation detail of `DiscretePid`, not exposed as their own netlist block kind.
  Achievable later by lifting the same `continuous_blocks::discrete_pid` recursion logic into a
  small standalone struct, not blocked on anything designed here.
- External reset (rising/falling/either-edge/level) for the integral action — the continuous
  PID has no reset input at all today either; not introduced here for parity with the sibling
  continuous block, not because it's architecturally hard.
- Saturation *wrapping* (vs. clamping) — `PidClamp`'s own existing fixed/dynamic clamp is
  reused unchanged; wrapping is a different, unrelated mechanism not built for the continuous
  PID either.
- Back-calculation anti-windup — the continuous PID only has the tentative-step/reject clamp
  method today; not extended here.
- Scalar-expansion for a vector input on a single-input discrete transfer function/PID — the
  continuous versions don't have this either; out of scope for the same reason.
