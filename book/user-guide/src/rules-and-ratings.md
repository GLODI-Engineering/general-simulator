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
R_ileg  kind=rule type=above  target=I(L1) limit=60 unit=A severity=error
R_bus   kind=rule type=outside low=390 high=410 target=V(bus) unit=V after=5m
R_leg   kind=rule type=forbidden_state switches=DSH,DSL action=stop severity=fatal
R_dead  kind=rule type=forbidden_state switches=DSH,DSL min_deadtime=200n
R_path  kind=rule type=required_path switches=DSH,DSL current=I(L1) i_min=1
```

`type=` is one of `above`, `below`, `outside`, `inside`, `forbidden_state`, `required_path` or
`switch_timing`.
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

## What a run produces

Events are written as they happen, to `<deck>.events.jsonl` and `<deck>.events.txt`, and at
`console` severity to stderr:

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
the run.

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
