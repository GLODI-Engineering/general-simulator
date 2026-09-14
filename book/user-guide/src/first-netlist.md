# Your first netlist

## The one-file convention

A `general-simulator` deck is one ordinary-looking text file. Every line is either a standard
SPICE element (`V`, `R`, `C`, `L`, `D`, ...) or a `kind=...` statement that declares a PWL device
model or a signal-domain block — there's no separate "device file" to maintain alongside the
netlist for the common case (`--devices <file>` still exists, for sharing one controller across
several netlists, but nothing below needs it).

Here's a complete deck: a voltage source charging a capacitor through an ideal diode and a
resistor.

```text
V1 a 0 5
D1 a b idealswitchmodel
D1 kind=ideal_diode g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.7 g_on=1
R1 b c 100
C1 c 0 1e-3
```

Line by line:

- `V1 a 0 5` — an ordinary 5V independent voltage source from node `a` to ground. Any real SPICE
  tool reads this line exactly as written.
- `D1 a b idealswitchmodel` — the diode's own element line: two terminals (`a`, `b`) and a model
  name (`idealswitchmodel`, an arbitrary label — nothing else in the file needs to reference it).
- `D1 kind=ideal_diode ...` — the second `D1` line is the *device-card* statement declaring what
  that model actually is: a piecewise-linear ideal diode with reverse-breakdown conductance
  `g_breakdown` (0 here — no reverse breakdown modeled), the breakdown voltage `v_breakdown`,
  off-state conductance `g_off`, forward threshold `v_th`, and on-state conductance `g_on`. This
  is what replaces SPICE's own `.model` card and its exponential `I = I_S(e^{V/V_T}-1)` — see the
  Introduction for why.
- `R1 b c 100` / `C1 c 0 1e-3` — an ordinary resistor and capacitor, exactly as any SPICE tool
  reads them.

## Running it

```bash
general-simulator first.cir --mode transient --tfinal 0.5 --dt 0.01
```

prints CSV to stdout, one row per resolved timestep:

```text
t,V(a),V(b),V(c),I(V1)
0.01,5,4.261261261261261,0.38738738738738737,-0.03873873873873874
0.02,5,4.264915859255481,0.7565017848036715,-0.035084140744518086
...
0.5000000000000002,5,4.299698388082447,4.269537196327209,-0.00030161191755269146
```
(the last row's `t` isn't exactly `0.5` — ordinary `f64` accumulation error from summing `0.01`
fifty times, not a bug worth chasing)

`t` is always first, then one column per circuit unknown (`V(node)` for every non-ground node,
`I(branch)` for a branch whose current the topology needs, like this source's own). `V(c)` climbs
from `0` toward its final value as the capacitor charges — see
[Reading the output](reading-output.md) for the full column-layout rules, including what changes
once a `kind=...` block is declared too.

## `--mode dc`

`--mode dc` solves a single operating point instead of stepping a transient — one row of output,
no `--tfinal`/`--dt` needed:

```bash
general-simulator first.cir --mode dc
```

For *this* particular deck, that actually fails (`Linear(SingularMatrix { pivot_column: 2 })`):
node `c`'s only connection is through the capacitor, and `--mode dc` doesn't give a capacitor the
traditional SPICE "open circuit at DC" treatment implicitly — a node whose sole path to the rest
of the circuit is a capacitor is a genuinely singular operating point here, not silently ignored.
Drop `C1` (or give `c` another DC path) and it resolves normally:

```text
t,V(a),V(b),I(V1)
0,5,4.257425742574257,-0.04257425742574257
```

Worth knowing before it surprises you on a real deck, not a corner case to memorize — most decks
with a controller in the loop (a PID, a PWM modulator) use `--mode transient` anyway, since a
block's own state only exists across a stepped run.
