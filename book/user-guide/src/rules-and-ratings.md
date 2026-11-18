# Rules and ratings: making a wrong run say so

A transient simulation almost never refuses to produce an answer. Gate two switches of the same
leg together and you get a waveform; run an inductor at six times its rating and you get a
waveform; saturate a core the model knows nothing about and you get a waveform. Every one of
them looks like the others, and the thing that makes them wrong is invisible in the output.

A rule is a question asked of every solver step, whose answer is recorded with the time it
became true. If nothing is asked, nothing is found — so a deck's rules are part of the deck,
written in the netlist beside the circuit they describe.

There are two syntaxes, and they compile to exactly the same thing:

- a **`kind=rule` card**, which says everything explicitly and can watch anything;
- a **rating on the device line**, which is shorter and is where a datasheet number belongs.

## Ratings on the device line

```text
Ls a b 36u i_max=60 i_sat=70 on_violation=console
```

That is one inductor and two limits. The fields:

| field | means |
|---|---|
| `i_max` | the element's current, in amps, may not exceed this magnitude |
| `i_sat` | the same, as a second threshold — a part rated for 60 A may saturate at 70 A |
| `v_max` | the voltage **across** the element may not exceed this magnitude |
| `on_violation` | what to do: `log`, `console`, `stop`, `pause` |
| `severity` | `info`, `warning`, `error`, `fatal` |
| `hold` | ignore an excursion shorter than this |

Three things about them are worth knowing before you write one.

**A rating is a limit on magnitude.** `i_max=60` means $|i| \le 60$, not $i \le 60$. A buck
inductor's current never goes negative and either reading would do; a dual-active-bridge
inductor's current is symmetric about zero, and a signed limit would watch $-70$ A go past a
60 A rating without a word.

**A rating is an error by default**, where a `kind=rule` card defaults to a warning. The number
came off a datasheet; exceeding it is not a matter of taste. The process exit code is therefore
`2` after a run that violated one, and `0` after a run that did not — enough for a script to
tell a trustworthy result from a plausible-looking one.

**A current rating needs a device that has a current.** Only `V`, `L`, `E` and `H` contribute a
branch current to the MNA unknowns; a resistor's, a capacitor's or an ideal switch's current is
computed from the solution rather than solved for, so there is no `I(name)` column to watch.
Writing `i_max=` on one of those is refused at load:

```text
line 9: device 'RL': a current rating needs a branch current, and device R does not have one
(only V, L, E and H do -- a resistor's or a switch's current is computed from the solution,
not solved for). Write a `kind=rule` card naming a current you do have, or probe this one first.
```

Refused rather than accepted-and-ignored, because in the end-of-run table a rating that was
never evaluated reads exactly like a rating that was never exceeded. For a switch current,
measure it with a `phys2sig` converter and write a card against that signal.

## `kind=rule` cards

A card can watch anything the waveform carries, and can express the conditions a rating cannot:

```text
R_ileg  kind=rule type=above  target=I(L1) max=60 unit=A severity=error
R_bus   kind=rule type=outside min=390 max=410 target=V(bus) unit=V after=5m
R_leg   kind=rule type=forbidden_state switches=DSH,DSL action=stop severity=fatal
R_dead  kind=rule type=forbidden_state switches=DSH,DSL min_deadtime=200n
R_path  kind=rule type=required_path switches=DSH,DSL current=I(L1) i_min=1
```

`type=` is one of `above`, `below`, `outside`, `inside`, `forbidden_state`, `required_path`,
`switch_timing`, `rate_of_change`, `body_diode`, `commutation_order` or `per_period`.
The first four take a `target=` naming any column the CSV has — a node voltage `V(node)`, a
branch current `I(element)`, a voltage across a pair `V(p,n)`, or a block output by its own name.

`forbidden_state` and `required_path` are the multi-component rules: they are about a
*relationship* between switches rather than a quantity.

- `forbidden_state switches=DSH,DSL` reports whenever more than `max_on` of them conduct at
  once (default 1) — shoot-through on a bridge leg, which is the failure that destroys hardware
  and which a simulation will otherwise simply absorb.
- The same rule with `min_deadtime=200n` reports a *handover faster than* 200 ns, including a
  handover with no dead time at all, where one switch turns on in the same step the other turns
  off. There is no all-off interval to measure there, so the check looks for the handover
  itself.
- `required_path switches=D1,D2 current=I(L1) i_min=1` reports the opposite mistake: a current
  of at least 1 A with no device left to carry it.

### `switch_timing`: when a switch changes, not what state it is in

```text
T_sh kind=rule type=switch_timing switches=DSH,DSL min_on=300n min_off=300n \
     f_min=18k f_max=22k
```

Three questions with one answer, because all three are read off the same record of resolved
transitions:

| field | reports |
|---|---|
| `min_on` | a conducting interval shorter than this |
| `min_off` | a non-conducting interval shorter than this |
| `f_min`, `f_max` | a measured switching frequency outside the band |
| `require_commutation` | a switch that held one state for the whole run (default: yes) |

**`require_commutation` defaults to yes**, and is the one to leave on. A gate that never
reaches its switch is the most common cause of a converter simulation that produces a plausible
wrong answer — a `ctrl=` naming the wrong block, a modulator whose output never crosses the
gate's threshold, a constant left at zero during debugging. What then runs is a different
circuit, usually one with a permanently open switch, which still charges and still settles. The
waveform of such a run looks entirely reasonable:

```text
[100.0000 us] ERROR DSW.T_sw  at 0.0000 transition(s)  limit 1.0000 transition(s)
              [DSW never commutated -- it stayed open for the whole run]
```

It also guards the other two checks: a switch that never commutated cannot have produced a
measurable on-time or frequency either, so a rule that asked for those and stayed quiet would
be reporting a pass it never established.

**Minimum on and off times** are about the device rather than the circuit. A real switch needs
driver propagation, bootstrap charging and current-sense blanking; a modulator asking for a
50 ns on-pulse from a device that needs 300 ns does not get a short pulse, it gets an
unpredictable one, and the duty cycle the simulation settled on describes a converter nobody
can build. The report carries the pulse that was actually delivered:

```text
[14.4000 us] WARN DSW.T_sw  at 4.4000 us  limit 6.0000 us
             [DSW was on for 4.4000e-6 s, 6.0000e-6 s required]
```

An interval already in progress at the first step is not judged — it began before anything was
watching, so its length is not a measurement.

**The switching frequency** is measured from the transitions, not read from the modulator's
`freq=`. With a closed loop it is an outcome rather than a setting: a hysteretic or
current-mode loop can double its frequency under one load and halve it under another, and the
magnetics, the losses and the EMI filter were all designed for a number nobody checked
afterwards. Two transitions make a period, and the window is the interval actually observed.

### `rate_of_change`: how fast a quantity may change

```text
R_cut kind=rule type=rate_of_change target=I(L1) max_slope=1e7 unit=A/s
```

The general form of the cut-current check. Open every switch in an inductor's path and the
current has nowhere to go; the solver answers with whatever voltage the stamp implies rather
than with an error, and the result is the enormous spike everyone has seen and that is
routinely blamed on the solver rather than on the netlist.

`required_path` catches that case when the switches that should have been conducting can be
named. This catches it from the other side, off the solution: a current step through an
inductance implies $v = L \, \mathrm{d}i/\mathrm{d}t$, so a bound on the slope is a bound on
that voltage. It is judged on magnitude, because a cut current collapses as violently as it
surges.

Pick the bound from the circuit, not from taste. A buck inductor with 9 V across 22 µH slews at
$9/22\,\mu \approx 4.1 \times 10^5$ A/s, so a bound of $10^7$ A/s leaves ordinary operation
alone and still catches a cut.

### `body_diode`: a switch that is off and conducting anyway

```text
B_dsl kind=rule type=body_diode switches=DSL currents=I(VSL) i_min=10
```

Not a fault by itself — it is exactly how a synchronous bridge freewheels during dead time, and
how every unsynchronised commutation is survived. It becomes a finding when the current is
large, because then it is either a sync FET whose channel was never turned on (so the losses
are the diode's, several times what the design assumed) or a path nobody intended.

`currents=` names one current per switch, in the same order, because **a switch's own current is
not an MNA unknown**. Only `V`, `L`, `E` and `H` carry one. The way to observe a switch current
is the standard one, and the engine suggests it when asked for such a current directly: put a
0 V source in series with the device and probe that.

```text
VSL mid sl 0
DSL sl 0 idealswitchmodel
```

Only a commanded-*off* switch can produce this finding, and that is not a limitation of the
rule but of what the question means: a switch commanded on is stamped as a plain resistance
conducting in both directions, so it cannot fail to conduct.

### `commutation_order`: the order switches take over the current in

```text
C_ord kind=rule type=commutation_order sequence=S1,S2,S3,S4
```

Multi-step commutation — a current-source inverter, a matrix converter, any four-step sequence
— is correct only if the transitions happen in the stated order. Out of order it still
simulates, and the answer is wrong in a way nothing in the waveform reveals.

The order is cyclic, and its phase comes from the first turn-on observed rather than from the
first entry of the list: a run that begins midway through a sequence has not got the order
wrong. After a departure the rule resynchronises to what actually happened, so one mistake
produces one finding rather than one per transition for the rest of the run.

### `per_period`: the measures that only mean something over a cycle

```text
P_vs  kind=rule type=per_period target=V(p,n) measure=integral period=10u \
      min=-1u max=1u unit=Vs
P_rms kind=rule type=per_period target=I(L1) measure=rms sync=DSH max=60 unit=A
```

Every check above judges an instant. The quantities that actually say whether a converter is
working are averages over a switching period, and the failure that motivated this one is a real
one: a transformer's magnetising current drifting from 40 A to 245 A over a run, with every
instantaneous rule passing throughout. Nothing was over a limit at any moment. What was wrong
is that the volt-seconds applied to the winding did not sum to zero over a period, so each cycle
left a little more flux than the last.

$$\int_{t}^{t+T} v \, \mathrm{d}t \approx 0$$

is the statement, and `measure=integral` with a tolerance is the rule.

| `measure=` | computes |
|---|---|
| `integral` | $\int x \, \mathrm{d}t$ over the period — volt-seconds, for a winding |
| `rms` | what a component is rated by, as opposed to its peak |
| `mean` | for a DC-link current or an output voltage |
| `abs_mean` | the mean of the magnitude, which is what a thermal estimate wants |

**What a period is** must be said, one way or the other:

- `period=10u` — stated outright, for an open-loop converter at a fixed frequency. The boundary
  is *split*: the sample at the end of the period is interpolated between the two steps either
  side of it, so the measure is over exactly one period. Closing at the step that noticed
  instead would carry a stray step's worth of integral into the result, which for a near-zero
  volt-second balance is not a small error.
- `sync=DSH` — each period runs from one turn-on of that switch to the next. With a closed loop
  the period is an outcome rather than a setting, so this is the only honest boundary. The
  fragment before the first turn-on is not measured: it is part of a cycle that began before
  anything was watching.

**On the numbers agreeing with `kind=measure`.** These are accumulated as the run goes rather
than computed over a stored trace — buffering a period per rule would be unbounded, and at
10 ns steps a 50 Hz period is two million samples. The per-segment forms are
`gs-waveform-measurements`' own, including the exact integral of a squared linear segment,
$\frac{\Delta t}{3}(v_0^2 + v_0 v_1 + v_1^2)$, rather than a trapezoid of the square. A test
asserts the streaming result and that library's batch result agree to twelve digits on the same
data, so a `kind=measure` RMS and a per-period RMS over the same window do not disagree in their
last digits by a quadrature rule nobody chose.

One number worth expecting: on a sampled square wave the transition between two samples
interpolates linearly through zero, and its contribution is exactly nothing — so a 60/40
asymmetric square wave of ±10 V integrates to $1.9\,T$ rather than $2\,T$ volt-seconds. That
is a property of the samples, not of the accumulator.

### Qualifiers

Available on any card, and `hold` on a rating:

| field | means |
|---|---|
| `hold` | the condition must hold this long before it is reported |
| `hyst` | leave only once the value has come back past the limit by this much |
| `after`, `before` | ignore the rule outside this window — `after=5m` skips start-up |
| `max_events` | stop writing records after this many; the summary still counts them all |
| `stop_after` | turn `action=stop` into a stop only on the Nth violation |

`hold` is the one to reach for first. A switch model can produce a 20 ns numerical spike that is
not a real overload, and a rule that reports it trains its reader to ignore the log.

## How a bound is named

There is one rule, and every rule type follows it:

> **A bound on whatever `target=` names is `min` or `max`. A bound on a quantity the card does
> not name in `target=` carries that quantity's prefix.**

So:

| rule type | `target=` names | its bounds |
| --- | --- | --- |
| `above`, `below` | the quantity | `max`, `min` |
| `outside`, `inside` | the quantity | `min` and `max` |
| `per_period`, `efficiency` | the quantity | `min` and `max` |
| `switch_timing` | switches | `f_min`, `f_max`, `t_min` ... |
| `body_diode` | switches | `i_min` |
| `zvs` | switches | `v_max` |
| `rate_of_change` | the quantity | `max_slope` |

The prefixed names are not inconsistencies. A card whose `target=` names *switches* cannot
write a bare `max=5`, because nothing would say which quantity — a frequency, a current, a
voltage — or which unit that 5 is in. `rate_of_change` is the subtle case: its `target=` is a
quantity, but the bound is on the **derivative** of that quantity, so `max_slope` earns its
prefix by naming something different from the target.

`limit=`, `low=` and `high=` used to exist and are now errors that name their replacement:

```text
error: line 12: rule 'EXPECT': field 'low' no longer exists -- write 'min'. Every bound on a
rule's own target is 'min' or 'max', on every rule type
```

They were removed rather than kept as aliases because three names for one concept is how
somebody writes the obvious thing and gets a parse error from a feature that otherwise works
perfectly.

## A deck is its own regression test

A rule whose bound is an *expectation* rather than a limit turns a deck into a test, because a
violated rule at `error` severity exits non-zero:

```text
EXPECT kind=rule type=outside target=V(out) min=4.0 max=5.0 after=3e-3 severity=error
```

On a buck that settles to 4.8 V:

```text
  V(out).EXPECT                passed
1 rule(s): 0 violated, 1 passed          → exit 0
```

With the expectation wrong on purpose (`min=9.0 max=10.0`):

```text
  V(out).EXPECT   ERROR  1 time(s), 2.0000 ms total, worst 4.4834 at 4.1838 ms
1 rule(s): 1 violated, 0 passed          → exit 2
```

`general-simulator deck.cir` in CI needs no wrapper script and no separate assertion file.
`after=` supplies the "after 5 ms" part of *"the bus is between 390 and 410 V after 5 ms"*, and
`hold=` is there for an assertion that should tolerate a brief excursion.

## The one check you do not have to ask for

Every rule above compares a value against a limit, and **every comparison against a `NaN` is
false** — `NaN > limit` and `NaN < limit` both. So a run whose state has stopped being made of
numbers finds no violations, summarises as `passed`, and exits 0, while its waveform file
carries a column of `NaN` that a plotting tool draws as a gap and a reader takes for a
rendering artefact.

So the finite check is on by default, on every step of every deck, and it needs no card:

```text
[100.0000 ns] FATAL BAD.finite  at NaN  limit inf
              [BAD is not a number, so every later value is meaningless and every
               rule comparing one is false. The run stops here.]
abandoned at step 0 (t = 1e-7 s): 'BAD' is not a finite number.
```

It covers both the solved state and the block outputs. A block output is the worse case: it is
not part of the solution, so nothing else examines it, and next step it reaches the circuit
through a gate or a source value. `sqrt` of a negative, `inv` of zero and `ln` of zero all
produce one, and none of them refuses.

The exit code is **4**, its own, because 3 and 4 mean opposite things to whatever reads them: 3
is a rule doing its job on a run that behaved as the deck said it would, and 4 is a run with no
result at all.

The rules engine has its own guard for the same problem: a non-finite sample counts as an
*unreadable* one, so a rule watching a quantity that has gone to `NaN` reports `NOT EVALUATED`
rather than `passed`. That matters for a block output nobody stops on, and for any other
program using the engine.

Why it is not opt-in, when everything else is: every other rule is written by someone who
suspected something. Nobody suspects a `NaN`, and a run that produced one has no result to
report, so there is nothing an opt-in would protect.

## What a run produces

Events are written as they happen, to `<deck>.events.jsonl` and `<deck>.events.txt`, and at
`console` severity to stderr. Both paths can be asked for instead — `--events <path>` and
`--events-text <path>`, independently; see
[Command-line flags](cli-reference.md#--events-asking-for-the-event-stream-instead-of-finding-it).

```text
[33.7782 us] ERROR L1.i_max           enter  3.5079 A  limit 3.5000 A
[34.8724 us] ERROR L1.i_max           leave  3.6528 A  for 1.0942 us (peak 3.6528 A)
```

The label is the component, then the rule. An `enter` carries the interpolated crossing time —
the moment the limit was passed, found between solver steps, not the step that noticed — along
with the step number, so the run can be reproduced to that step. A `leave` carries how long the
excursion lasted and its peak.

Then the table, which is the part worth reading first:

```text
--- rules ---
  L1.i_max                     ERROR   3 time(s), 4.8928 us total, worst 3.8190 A at 44.1000 us
  L1.i_sat                     passed
  C1.v_max                     ERROR   2 time(s), 195.5163 us total, worst 5.5207 V at 96.0000 us
3 rule(s): 2 violated, 1 passed
```

Every rule appears, including the ones that found nothing. `passed` means the question was asked
and the answer was no; `NOT EVALUATED` means the quantity was never there to read, which is a
problem with the rule rather than with the circuit.

Exit codes: `0` nothing above a warning, `2` something at `error` or `fatal`, `3` a rule stopped
the run, `4` the run went non-finite and was abandoned.

## Stopping, adjusting, and carrying on

`action=stop` (or `on_violation=stop`) ends the run at the violation — and still writes the
waveform, the event log and, if asked, the checkpoint. So the interesting workflow is:

```bash
general-simulator deck.cir --mode transient --tfinal 3e-4 --dt 1e-7 \
    --checkpoint-out run.ckpt
# → exit 3: stopped by rule 'L1.i_max'

# raise the limit, or widen the dead time, in deck.cir — then:
general-simulator deck.cir --mode transient --tfinal 3e-4 --dt 1e-7 \
    --resume run.ckpt
```

Editing a limit does not invalidate the checkpoint. A checkpoint refuses to load into a deck it
did not come from, and that identity is a hash over the deck's statements — but rule cards and
device ratings are removed before the hash is taken, precisely so that adjusting a limit and
resuming is possible. Editing the *circuit* does invalidate it, which is correct: that state
belongs to the circuit it came from.

The resumed run continues bit-identically: stopping at a violation and resuming reproduces the
uninterrupted trace exactly, row for row.
