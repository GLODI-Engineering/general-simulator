# Losses: what a converter is really limited by

A design that passes every rating can still be thermally impossible. Current and voltage limits
say whether a device survives an instant; loss says whether it survives the hour. This chapter
covers the loss the tool can compute **exactly**, which is conduction loss in a switch's channel.

## `P_cond(NAME)` is a quantity, not a new rule type

A switch's conduction power is exposed as a named quantity, so every rule type, every qualifier
and every per-period measure already applies to it:

```text
LOSS_H kind=rule type=per_period target=P_cond(DSH) measure=mean period=2e-3 max=5 unit=W
```

That is the whole feature. There is no `type=conduction_loss`, because there did not need to be
one — a loss is a quantity, and [per-period measures](rules-and-ratings.md) already average a
quantity over a cycle.

An instantaneous bound works the same way, and is a different question — peak dissipation rather
than average:

```text
PEAK_H kind=rule type=above target=P_cond(DSH) limit=200 unit=W severity=error
```

## Nothing has to be declared, and nothing is restated

```text
DSH in mid idealswitchmodel
DSH kind=ideal_switch r_on=0.02 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.5 g_on=100 gate=block ctrl=GH
```

Those two lines are all `P_cond(DSH)` needs. The device line gives the node pair, the
`kind=ideal_switch` line gives `r_on`, and the rule card restates neither — an `r_on` written
twice can drift from the device's, and would then report a loss the circuit does not have.

In particular **no sense source is required.** A switch commanded on is stamped as a plain `r_on`
resistance between its declared nodes, so its current is not an unknown the solver has to carry —
it is exactly computable:

$$i = \frac{V_a - V_b}{r_{on}}, \qquad p_{cond} = \frac{(V_a - V_b)^2}{r_{on}} = i^2 r_{on}$$

Checked against a zero-volt sense source in series with the switch of a buck, the worst relative
disagreement across a run was $2.7 \times 10^{-9}$.

## The state gates the expression, and that is the whole correctness argument

$p_{cond}$ is **exactly zero while the switch is commanded off**, and not because that is
convenient. While off, the device is no longer a resistance and the voltage across it is the full
blocking voltage, so the same arithmetic gives nonsense: on the run above it computed 1240 A
against a true 0 A.

Two consequences worth knowing:

- A per-period **mean** of `P_cond` comes out as the duty-weighted average automatically, because
  the off time contributes zeros. You do not have to tell the rule what the duty is — which
  matters most in a closed loop, where the duty is an outcome.
- **Body-diode conduction is not counted.** While a switch is commanded off, any current flows
  through its body diode, where the loss is $v_f i + i^2 / g_{on}$ — a different expression on a
  different state. `P_cond` is the channel only. A synchronous rectifier whose channel is never
  turned on therefore reports *zero* conduction loss while dissipating substantially in its
  diode, which is exactly the failure
  [`type=body_diode`](rules-and-ratings.md) exists to catch, and the two rules belong together on
  a sync leg.

## A worked check: the duty split across a synchronous leg

At duty $D$, the high-side switch carries the inductor current for $D$ of each period and the
low-side for $1 - D$, both at the same $r_{on}$. So their conduction losses must be in the ratio

$$\frac{P_{cond,L}}{P_{cond,H}} = \frac{1 - D}{D}$$

On the test deck ($V_{in} = 12$ V, $D = 0.4$, $R = 5\ \Omega$, $r_{on} = 20$ m$\Omega$, 200 ns
dead time) the measured figures are 0.0069 W high-side and 0.0105 W low-side, a ratio of 1.52
against the required 1.50. The hand estimate from the ideal duty, $0.4 \times 0.96^2 \times 0.02 =
0.00737$ W and $0.6 \times 0.96^2 \times 0.02 = 0.01106$ W, sits a few percent above the measured
values because it ignores the dead time and the conduction drops.

The *ratio* is the check worth making. An absolute figure can be right for both switches while
being attributed to the wrong one; the ratio cannot.

## A warning about the complement

Writing a synchronous leg's low-side gate as PWM of $1 - D$ rather than as the complement of the
high-side's PWM makes both switches conduct together on every cycle. The first version of the test
deck did exactly that, and the conduction loss came out as **721 W** in a 12 V converter.

That number was correct. 12 V across two 20 mΩ channels is 300 A, and
$300^2 \times 0.02 \times 0.4 = 720$ W. Use the complement output the modulator already provides:

```text
PWMH kind=pwm freq=50000 in=DUTY red=200e-9 fed=200e-9
GH kind=sig2phys domain=voltage in=PWMH
GL kind=sig2phys domain=voltage in=PWMH_comp
```

and put a [`type=forbidden_state`](rules-and-ratings.md) rule on the leg so the deck says it for
you.
