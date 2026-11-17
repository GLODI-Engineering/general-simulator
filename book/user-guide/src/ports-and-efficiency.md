# Ports and efficiency: is this a converter at all?

Every rule in [Rules and ratings](rules-and-ratings.md) bounds one quantity: a current, a
voltage, a dead time, a volt-second balance. None of them can ask the question that matters most
about a converter, which is whether it conserves energy.

That question has an awkward property: **every individual waveform can sit inside its own limit
while the circuit as a whole invents energy.** A transformer stamped with the wrong turns ratio,
a dependent source with the wrong gain, a coupling coefficient out of range, a port wired to the
wrong node — each produces a run that completes, a waveform that looks plausible, and an answer
that is wrong by a factor. Nothing in the trace says so.

## Why power needs a declaration

The rules engine reads *named* quantities. `I(L1)` is a name the solver resolves; so is
`V(out,0)`. Power is

$$p = v \cdot i$$

which is a product of two names, and the engine has no arithmetic. There was nothing to write in
a rule's `target=`.

Rather than give the rule grammar an expression language — which would have turned every field
into a parser, and every error message into a parser's error message — power is declared as an
object:

```text
P_IN  kind=port v=(in,0)  i=-VIN
P_OUT kind=port v=(out,0) i=VSENSE
```

A **port** is a boundary with a voltage across it and a current through it. `v=(p,n)` is the node
pair the voltage is measured across, where `0` is ground. `i=` names the element whose branch
current is the port's current — one the solver actually solves a current for, which means a `V`,
`L`, `E` or `H`. To put a port anywhere else, put a zero-volt source in series and name that:

```text
VSENSE out load 0
R1 load 0 5
```

A zero-volt source changes nothing about the circuit and gives the solver a branch to report.

Once declared, a port's name **is** a quantity, so every existing rule type can watch it. A
power limit is an ordinary `type=above` card:

```text
P_SW kind=rule type=above target=P_IN max=50 unit=W severity=error
```

## The sign, and the one minus you have to write

A port's power is the plain product

$$p = V(\text{positive}, \text{negative}) \times I(\text{branch})$$

with the solver's own sign, and no hidden negation anywhere. The one control is a leading minus
on `i=`, which negates the branch current.

You need it on a source and not on a sense element, and the reason is worth seeing once. The
solver reports a branch current in the **passive convention** — positive *into* the element's
first node. Solve `VIN in 0 12` feeding `R1 in 0 12` and you get

```text
t,V(in),I(VIN)
0,12,-1
```

`I(VIN) = -1 A`, while 1 A plainly flows out of `in` and through the resistor. A source's current
leaves its positive terminal, so an input port negates it:

```text
P_IN kind=port v=(in,0) i=-VIN     →  12 × 1 = +12 W
```

A series sense element is the other way round: current *enters* its positive terminal, so
`I(VSENSE)` is already the load current and no minus is wanted:

```text
P_OUT kind=port v=(out,0) i=VSENSE →  V_out × I_load, positive
```

Those two lines are the whole vocabulary. A port declared the wrong way round reports a negative
power, and an efficiency rule with `min=0` reports *that* rather than quietly inverting the
answer.

## The efficiency rule

```text
ETA kind=rule type=efficiency in=P_IN out=P_OUT min=0 max=1 period=2e-3 severity=error
```

$$\eta = \frac{\overline{p_\text{out}}}{\overline{p_\text{in}}}$$

Both powers are averaged over the same period, and `max=1` is the card that says "this is a
converter": above it, energy is being created.

The quantity is deliberately a **ratio** rather than a bounded difference between the two powers.
A difference needs a tolerance, and a tolerance needs a reference — a percentage of the input, of
the larger side, of the rated power — and each choice makes a different claim. On a converter at
5% load, 2% of the input power is a different statement from 2% of the rating, and neither is the
one anyone means. A ratio is dimensionless and self-normalising, so $\eta \le 1$ is the same
correct claim at full load and at no load.

It also catches the lossless mistakes that an efficiency *floor* would miss. A transformer with
the wrong turns ratio does not look lossy — it looks like a gain. A band of

```text
ETA kind=rule type=efficiency in=P_PRI out=P_SEC min=0.99 max=1.01 sync=DSW
```

across such an element reports it.

Both quantities are averaged because instantaneous power changes sign inside a switching cycle:
the useful statement is about the cycle, not the instant. `period=` and `sync=` work exactly as
they do for [per-period measures](rules-and-ratings.md), and the same rule applies — with a
closed loop, use `sync=` so the period is the one the circuit actually produced.

## The one thing this rule does not know: stored energy

**A per-period efficiency can legitimately exceed 1.** This is not a defect in the rule and not
energy creation, and if you write `max=1` without knowing it you will be told your working
converter is broken.

$\eta$ accounts for no *stored* energy. While the output filter rings, the capacitor spends half
of each ring cycle delivering into the load energy that the input supplied on an earlier cycle —
so for that half-cycle the load genuinely receives more power than the source provides.

The buck in the test suite shows it plainly. With $L = 1$ mH and $C = 100\ \mu$F,

$$f_0 = \frac{1}{2\pi\sqrt{LC}} = 503\ \text{Hz}, \qquad T_0 = 1.99\ \text{ms}$$

and measured over **one switching period** (20 µs) it reports efficiencies up to 1.29 during
start-up. Over a 1 ms window — half a ring period — it reports 1.36. Over 2 ms, one full ring
period, it passes across the entire run including start-up.

So:

> **Choose an averaging window long compared with the slowest energy-storage cycle in the
> circuit, not with the switching period.**

If you want the switching-period resolution anyway, use `after=` to start the rule once the
transient has decayed — but note that an underdamped output filter rings for many milliseconds,
so `after=` has to clear the ringing and not merely the first cycle.

## Ports are not sugar for a product block

The netlist can already compute a power without any of this: probe the node and the branch into
the signal domain and multiply them.

```text
VPROBE kind=phys2sig node=in
IPROBE kind=phys2sig branch=VIN
NEG kind=const value=-1
P_PRODUCT kind=product inputs=VPROBE,IPROBE,NEG
```

That works, and it is not equivalent. A block output is evaluated with a one-step lag, so the
product trails the port by one step and only converges to it as the step shrinks. Measured on the
buck above, with start-up excluded, the product's disagreement with the port was

| step | $P_\text{product} / P_\text{port}$ | error |
| --- | --- | --- |
| 400 ns | 0.9763 | 2.37% |
| 200 ns | 0.9882 | 1.18% |
| 100 ns | inside 1% | — |

halving with the step: first order, which is exactly what a one-step lag gives. The port reads
the solved values at the step itself, so it has no lag to converge away.
