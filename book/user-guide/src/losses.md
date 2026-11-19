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
PEAK_H kind=rule type=above target=P_cond(DSH) max=200 unit=W severity=error
```

## Nothing has to be declared, and nothing is restated

```text
DSH in mid idealswitchmodel
DSH kind=ideal_switch r_on=0.02 body_g_breakdown=0 body_v_breakdown=-100 body_g_off=0 body_v_f=0.5 body_g_on=100 gate=block ctrl=GH
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

# Was it really ZVS?

```text
ZVS kind=rule type=zvs switches=DSH,DSL v_max=5 severity=error
```

At each turn-on, report the voltage that was still standing across the switch. You name only the
switches — the voltages come from each switch's own declared node pair, because two parallel lists
in a deck can fall out of step, and the failure mode would be a rule silently reporting the wrong
switch's voltage.

A **bound** rather than a yes-or-no, because partial ZVS is the interesting case. A converter
commutating to 40 V on a 400 V bus is doing 1% of the hard-switching loss and is a design that
works; one reaching 380 V will fail thermally. A boolean cannot tell those apart.

## One leg, both answers

A synchronous buck at $D = 0.4$ with dead time gives both results at once, which is worth running
yourself:

```text
ERROR DSH.ZVS_H   at 12.5071 V   limit 5.0000 V   [DSH turned on with V(in,mid) still across it]
      DSL.ZVS_L   passed
```

The two switches of the *same leg* commutate completely differently:

- **DSH** turns on into the full bus. Nothing has discharged the switching node, so it sees 12 V
  plus the freewheeling diode's drop. Hard switching.
- **DSL** turns on *behind its own body diode*, which has been freewheeling throughout the dead
  time, so the voltage across it is already down to a diode drop. That is textbook ZVS, and it is
  free — it is the reason synchronous rectification with dead time soft-switches the low side
  without any resonant element.

Tightening the low side's bound to `v_max=0.01` shows the pass is a real measurement and not a
skipped one:

```text
ERROR DSL.ZVS_L   at 0.5080 V   limit 0.0100 V
```

0.508 V is the body diode's own forward drop — $v_{th} = 0.5$ plus $i/g_{on} = 0.96/100$, agreeing
to a millivolt.

## Why there is no switching-energy rule

The obvious companion to this rule would be the energy per edge:

$$E_{sw} = \int_{\text{edge}} v\,i\,\mathrm{d}t$$

It is not offered, because it is not measurable at a step size anyone can afford. Measured on a
400 V hard-switched leg:

| deck | `dt` | $E_{sw}$ measured |
| --- | --- | --- |
| no $C_{oss}$ | 1 ns | 1.592e-9 J |
| no $C_{oss}$ | 250 ps | 3.968e-10 J |
| 400 pF across the switch | 1 ns | 1.634e-9 J |
| 400 pF across the switch | 250 ps | 5.681e-10 J |

against $\tfrac{1}{2} C V^2 = 3.2 \times 10^{-5}$ J.

Quartering the step quarters the figure, in **both** decks. Without $C_{oss}$ that is expected —
an ideal switch commutates in a single step, so there is no interval to integrate over and the
number is a discretisation artifact converging to zero. With $C_{oss}$ it is still proportional to
the step and still five decades low, because the discharge time constant is

$$\tau = r_{on} C_{oss} = 0.01\ \Omega \times 400\ \text{pF} = 4\ \text{ps}$$

and at any step above that the discharge completes inside one step, so the dissipation is absorbed
algebraically instead of resolved. Resolving it needs $\Delta t \ll 4$ ps — millions of steps
across one commutation, which is exactly what the CLI's own `--out-every` help text has always
said about verifying ZVS.

So the tool reports the **voltage**, and $\tfrac{1}{2} C_{oss} V^2$ is your multiplication, with a
$C_{oss}$ you have from a datasheet and the simulator does not have at all. That is a better
division of labour than a number that looks like an energy and is really a step size.

## Why there is no reverse-recovery rule either, and what to multiply instead

Reverse recovery is the charge $Q_{rr}$ that flows backwards through a diode after its current
reaches zero, and in a hard-switched leg with a slow body diode it is the dominant switching
loss. There is no rule for it, for the same reason and with the same division of labour.

**Measuring the charge is the switching-energy problem again.** $Q_{rr}$ is
$\int i\,\mathrm{d}t$ over the recovery interval, and this engine's diode is piecewise-linear
and ideal: it conducts or it does not, and the commutation happens inside one step. There is no
interval to integrate over, so the figure would be a discretisation artifact exactly as
$E_{sw}$ is above.

**What the engine does know is the voltage**, and it already reports it. A diode recovers into
the voltage that appears across it when the opposing switch turns on — which is the same
instant, and the same voltage, that [`type=zvs`](#was-it-really-zvs) measures on that switch. On
a 400 V hard-switched buck leg at 100 kHz:

```text
ZVS kind=rule type=zvs switches=DSW v_max=5 severity=warning
```

```text
[10.0000 us] WARN  DSW.ZVS  at 400.8791 V  limit 5.0000 V  [DSW turned on with V(vin,sw) still across it]
[20.0010 us] WARN  DSW.ZVS  at 400.9561 V  limit 5.0000 V
[30.0010 us] WARN  DSW.ZVS  at 401.0291 V  limit 5.0000 V
...
```

Six commutations in 60 us — one per switching period — each with the voltage the freewheeling
diode `D1` recovered into. So

$$E_{rr} = Q_{rr} V \approx 80\,\text{nC} \times 400.9\,\text{V} = 32.1\ \mu\text{J}$$

per commutation, or 3.2 W at 100 kHz, with a $Q_{rr}$ from a datasheet the simulator does not
have. The multiplication is yours; the voltage, the count and the instants are the engine's.

**Worth noticing in those numbers:** the voltage is not 400 V. It is 400.88 V and rising, because
the freewheeling node sits slightly below ground while the diode conducts, so the switch — and
the diode — stand off a little more than the bus. A hand calculation assuming the nominal bus is
0.2% low on the first commutation and drifting. That is precisely the kind of thing worth
measuring rather than assuming, and it is why the tool reports the voltage instead of a product
it would have to take a datasheet number for.

# Does the loss model describe the circuit at all?

Every rule so far bounds one device's loss. None of them can answer the question that decides
whether the whole budget is worth anything: **is the loss model accounting for the power the
circuit is actually losing?**

```text
P_IN  kind=port v=(in,0)  i=-VIN
P_OUT kind=port v=(out,0) i=VSENSE
CLOSE kind=rule type=loss_closure in=P_IN out=P_OUT devices=DSW,D1 max=0.002 \
      period=2e-3 after=6e-3 severity=error
```

$$\text{unaccounted} = \frac{\left|\,\overline{p_\text{in}} - \overline{p_\text{out}}
- \sum_k \overline{p_k}\,\right|}{\overline{p_\text{in}}}$$

The measured deficit between the ports minus everything the model claims to explain, as a
fraction of the input power. You name the devices; their conduction quantities are filled in, so
switches and diodes are listed the same way and a name the netlist does not have is refused
rather than quietly counted as zero.

Scaled by input power rather than bounded in watts for the same reason efficiency is a ratio: a
0.2 W residual means something very different on a 4 W converter and a 4 kW one, and a deck
should not have to restate its own power level inside a tolerance. `max` is therefore a fraction
in $0..1$, and a `max=5` is refused — it would read as 5 W and would never fire.

## Why this is the most useful rule in the chapter

**Every per-device figure can be individually correct while the model explains almost nothing.**
On the buck in the test suite:

| | deficit $\overline{p_\text{in}} - \overline{p_\text{out}}$ | modelled | unaccounted |
| --- | --- | --- | --- |
| `devices=DSW,D1` | 0.2433 W | 0.2391 W | **0.097%** |
| `devices=DSW` | 0.2433 W | 0.0165 W | **5.62%** |

The switch's 0.0165 W is a *true number about the switch* in both rows. In the second it is also
7% of the real loss, because the freewheeling diode dissipates 0.2227 W — **93% of the
conduction loss in this converter.** A loss report counting only switches would have made a
synchronous rectifier look like a pointless complication.

That 0.097% is cross-checked rather than trusted: integrating the same waveform in Python with a
plain trapezoid gives the same deficit, the same modelled total and 0.0969% unaccounted, from a
completely different code path than the engine's interpolating accumulator.

## The same stored-energy caveat as efficiency

`after=6e-3` in the card above is not decoration. The closure residual is **not** small until the
circuit settles:

| window | unaccounted |
| --- | --- |
| 2 – 4 ms (still ringing) | 3.48% |
| 4 – 6 ms | 0.51% |
| 6 – 8 ms (settled) | 0.097% |

Nothing is wrong with the model in the first row. The balance accounts for no *stored* energy, so
while the output filter rings, part of the deficit is energy going into $L$ and $C$ rather than
into heat — exactly the effect that lets a per-period efficiency exceed 1. Use `after=` to start
the check once the transient has decayed, and remember that an underdamped output filter rings
for many milliseconds.

A residual that *stays* large after settling is the real finding, and it means one of three
things: a device is missing from `devices=`, a loss mechanism is not modelled at all (an ESR, a
core loss), or a port is wired to the wrong node.
