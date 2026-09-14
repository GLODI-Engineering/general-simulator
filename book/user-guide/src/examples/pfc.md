# Three-phase PFC active front end

A three-phase active-front-end power-factor-correction (PFC) rectifier exercises more of
`general-simulator` at once than any other example in this guide: the
[Clarke/Park coordinate transforms](../coordinate-transforms.md), a synchronous-reference-frame
grid synchronizer, decoupled dq current loops, a real six-switch switching bridge, and a dynamic
PID anti-windup clamp — all closing through the same real circuit at the same time. This chapter
follows the staged approach used throughout this project's own development: validate the control
chain in isolation first (Stage 1), then close the loop through a real switching power stage
(Stage 2). Both stages are adapted from a real, ongoing internal three-phase PFC experiment —
described here generically, without naming or linking that private archive — kept honest about
what currently works and what doesn't.

## Stage 1 — ideal control chain, no power circuit

Before wiring any switching device, the control chain itself is validated against hand-derived
results, with no power circuit at all — the same discipline you'd apply unit-testing a
controller before ever connecting it to real hardware.

**Part A — the synchronizer.** A balanced three-phase grid voltage is built directly from an
integrated angle (`kind=vco`), fed through [`kind=clarke`](../coordinate-transforms.md) to get
`alpha`/`beta`, then [`kind=anglewrap`](../component-reference.md#waveform-arithmetic-functions-binary)
to recover the angle estimate `theta_hat`, and finally
[`kind=park`](../coordinate-transforms.md) using that estimate. Because `anglewrap` is an
instantaneous algebraic function (`atan2`, not a filtered/tracking PLL), there is no settling
transient to account for — the recovered `d`-axis voltage should match the grid's real peak
amplitude, and `q`/zero-sequence should hold at `0`, at *every* instant, even across a
discontinuous frequency step. That is exactly what was measured: across a 50 Hz -> 52 Hz step,
the `d`-axis output held its expected peak to floating-point precision (residual `q`/zero-sequence
values around `1e-13`-`1e-14`), immediately before and after the discontinuity.

**Part B — the current loop.** A standard IMC/pole-cancellation PI design closing a decoupled dq
current loop around an idealized `L`-`R` plant, using
[`Signal::BlockPrev`](../signals.md#when-to-reach-for-prev) (`prev:`) to close the loop around
the PI block itself with a one-sample delay — the same delay a real digital controller's own
feedback path has. The simulated step response matched the hand-derived closed-form
$i_d(t) = i_{d,ref} \cdot \left(1 - e^{-\omega_c (t - t_0)}\right)$ to within 0.63% of the
commanded step at the finest tested step size, with the residual error itself characterized as a
genuine, shrinking discretization artifact of the one-sample delay (not unexplained noise) —
roughly halving each time the timestep halved.

An earlier revision of this same current-loop deck had a real bug, caught by checking `iq`
against its own `0` expectation rather than assuming a plausible-looking result was correct: the
cross-axis decoupling compensation was wired into a plant transfer function that had no physical
coupling of its own to cancel, so `iq` settled to a nonzero value instead of `0`. The fix removed
blocks rather than adding a correction, once the algebra was reworked by hand: a *correctly*
decoupled coupled plant reduces exactly to the plain, uncoupled $1/(Ls+R)$ transfer function used
in Stage 1's test — so wiring the coupling and its exact cancellation as two separate blocks was
unnecessary, not merely simplifiable.

## Stage 2 — the real switching bridge

Stage 2 replaces the idealized plant with a real six-[`kind=ideal_switch`](../pwl-devices.md#kindideal_switch-on-resistance-plus-body-diode)
two-level bridge: three grid sources through series `R`-`L` AC inductors into three
half-bridge legs, a DC bus capacitor and load, each leg PWM'd from the
[PWM Modulator](../component-reference.md#pwm-modulator-1)'s own complementary-output pair, and
an outer DC-bus voltage loop feeding the Stage 1 current loops' `id` reference through a 20 ms
soft-start ramp.

**Bugs found and fixed along the way** (each root-caused against a deliberately isolated
diagnostic, not guessed and re-tuned around):

1. **Fixed-target duty normalization.** `duty = 0.5 + v_{cmd}/V_{target}` badly mismatched the
   bridge's actual achievable pole voltage while the bus was still near `0V` during startup.
   Fixed by normalizing against the *measured* bus voltage instead, safely clamped to avoid a
   divide-by-zero before any charge exists on the bus.
2. **Startup inrush at full mains scale.** At mains voltage/power levels, the AC-side `R`-`L`
   branch provides very little natural current limiting against a stiff grid with near-zero
   initial bus counter-voltage. Real hardware solves this with dedicated precharge circuitry,
   out of scope here — this example instead uses a deliberate bench-scale simplification
   ($V_m = 50\,\text{V}$ peak phase, $V_{dc}$ target $120\,\text{V}$), not full mains ratings.
3. **A fixed PID anti-windup clamp against a dynamic achievable range.** Even at bench scale, a
   clamp sized for the eventual bus target was hugely oversized relative to the actually
   achievable pole voltage while `Vdc` was still low, so the PID's own anti-windup never
   engaged even though the real plant was already saturated. Fixed with a dynamic anti-windup
   bound (`clamp_lo_in=`/`clamp_hi_in=`, read fresh from the signal graph every step instead of
   fixed at build time) tied to the actual, still-rising bus voltage.
4. **The anti-windup clamp bounded the wrong signal.** Fix 3 bounds the PID block's own output
   (`VD_PI`/`VQ_PI`) to `[-V_{dc}/2, +V_{dc}/2]` — but the command actually sent to the bridge is

   $$V_{D,cmd} = e_d + \omega L_{ac} \, i_{q} - v_{d,PI}$$

   a `sum` block *downstream* of that clamp, which also adds in the grid-EMF feedforward term
   $e_d$ (about $50\,\text{V}$ here) and the cross-axis decoupling term. Nothing ever bounded
   $V_{D,cmd}$/$V_{Q,cmd}$ themselves, and $e_d$ alone already exceeds the bridge's actual
   achievable pole voltage for most of the run. Measured on a 30 ms transient
   (`--dt 1e-5 --tfinal 0.03`) before this fix: `DUTY_A`/`DUTY_B`/`DUTY_C` sat outside `[0,1]`
   for 82.0% / 78.1% / 83.9% of the run, `iq` reached `-36.38 A` against a `0 A` command, and
   `Vdc` swung to `-18.99 V` — a physically meaningless negative bus voltage.

   The fix uses [`kind=limit`](../component-reference.md#waveform-arithmetic-functions-ternary-if-limit) — the block-graph
   clamp that takes all three of its arguments (`x`, and the two bounds) as ordinary *signals*,
   not fixed netlist constants, so the bound can itself be a live, time-varying expression
   computed from the bus voltage. A first attempt clamped $V_{D,cmd}$ and $V_{Q,cmd}$
   *independently*, each to $[-V_{dc}/2, +V_{dc}/2]$: this measurably helped (duty saturation
   dropped to roughly 50% of the run) but did not fully fix it, because clamping the two axes
   independently still lets their vector sum $\sqrt{v_d^2 + v_q^2}$ — the quantity the dq→abc
   inverse transform actually projects into each phase — exceed the bridge's true achievable
   range. The version kept here instead scales $(v_d, v_q)$ down *together*, preserving their
   ratio, whenever their combined magnitude exceeds $V_{dc}/2$:

   $$\text{scale} = \min\left(1,\ \frac{V_{dc}/2}{\sqrt{v_{d,raw}^2 + v_{q,raw}^2}}\right)
   \qquad V_{D,cmd} = v_{d,raw} \cdot \text{scale}, \quad V_{Q,cmd} = v_{q,raw} \cdot \text{scale}$$

   built from `kind=hypot`, `kind=min`, `kind=pow`, and `kind=limit` (the last only to floor the
   magnitude away from `0` before the reciprocal, avoiding a divide-by-zero at the first step).

**Result of fix 4, measured on the same 30 ms transient:**

| quantity | before fix 4 | after fix 4 (vector-magnitude clamp) |
|---|---|---|
| `DUTY_A`/`B`/`C` outside `[0,1]` | 82.0% / 78.1% / 83.9% of the run | **0.0% / 0.0% / 0.0%** |
| `iq` range | `-36.38 A` to `0.00 A` | `-31.08 A` to `0.00 A` |
| `Vdc` range | `-18.99 V` to `52.02 V` (goes negative) | `0.00 V` to `50.31 V` |
| `Vdc` at `t = 30 ms` (target `120 V`) | `46.77 V` | `17.87 V` |

## Honesty about current status

Fix 4 is a real, measured improvement for the specific bug it targets: the bridge's commanded
pole voltage now genuinely stays inside its achievable range for the entire run, and the DC bus
no longer swings to a physically meaningless negative voltage. **It is not, however, a fix for
full closed-loop convergence, and this chapter is not claiming one.** `iq` still diverges to
tens of amps instead of holding near its `0 A` command, and `Vdc` still does not track its
soft-start ramp toward `120 V` — the deeper current-loop/decoupling bug this experiment already
flagged as unresolved was being *masked* by command saturation, not *caused* by it, and remains
open after this fix. The synchronization chain itself (`kind=clarke`/`kind=anglewrap`/
`kind=park`) stays provably correct throughout every attempt, including this one — the bug is
confined to the current-loop/decoupling/anti-windup interaction under the bridge's real,
non-ideal, delayed dynamics, not the coordinate-transform machinery.

This example is deliberately left here as an honest in-progress record rather than presented as
a working closed-loop PFC rectifier. It does not yet appear in the
[Validation chapter](../validation.md) — see that chapter's "What isn't covered yet" section —
because it has not converged, and a validation chapter that presented a non-working result as
working would not be trustworthy. The likely next step, unchanged by this session's fix: verify
the decoupling and grid-feedforward algebra against the real circuit's actual (non-ideal,
delayed) transfer function, rather than the idealized one Stage 1's control-chain test assumed.
