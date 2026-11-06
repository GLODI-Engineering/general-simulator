# Reading the output

`general-simulator` produces one waveform per run — the resolved circuit unknowns plus every
declared block's own output(s), one row per point (`--mode transient`) or a single row
(`--mode dc`) — serialized as either CSV (the default) or a SPICE rawfile (`--format raw`; see
[Command-line flags](cli-reference.md)). Both are built from the *same* in-memory data, so the
column layout described below applies to both, and running the same netlist with each `--format`
is directly comparable row for row, value for value (see `tests/raw-output-python/` for exactly
that comparison against two independent readers).

## Column layout

- **`t`** (CSV) / **`time`** (rawfile — SPICE's own conventional name for the leading axis,
  which is why this crate's own writer renames it there; see `raw_format.rs`'s module doc
  comment) is always first.
- Then one column per circuit unknown, in the order `general-mna`'s solver assigned them:
  `V(node)` for each non-ground node voltage, `I(branch)` for each branch current the topology
  requires (e.g. an independent voltage source's own current).
- Then one column per declared block, by name — a `kind=pid` block named `PID1` is a column
  literally named `PID1`, holding that block's own output value each step.
- **A block's *extra* named outputs** — a multi-output block (`kind=cscript` with `outputs=`,
  `kind=clarke`/`kind=park`/... and the other coordinate transforms, `kind=pmsm`, `kind=pwm`/
  `kind=pspwm`) registers more than one output beyond its own block name; each extra output gets
  its own column too (its own name, or an auto-generated one — see each block's own doc entry in
  the Component Reference for its exact naming convention).
- **A vector-valued signal** (a `kind=statespace`/`kind=tf` block whose declared shape is a
  vector, not a scalar) expands into one column per element: `NAME[0]`, `NAME[1]`, ... rather
  than one column holding the whole vector. Arity is fixed for the whole run once a block
  declares it — see `book/dev-guide/src/vector-signals.md`.

## What time a row's values belong to

**The first row is $t = \Delta t$, not $t = 0$.** The initial condition is the run's input, not
one of its results, so it is never emitted; `--mode transient --tfinal 1e-3 --dt 1e-4` writes ten
rows, the first stamped `0.0001`. Any measurement over the whole trace is therefore taken over
$[\Delta t, t_\mathrm{final}]$ — which is where the ~0.01 offset in
[Measurements](measurements.md)' hand-derived `ramp.cir` example comes from.

**Every value in a row is the value at that row's own `t`** — each circuit unknown, and every
signal-domain block that does not read the circuit. There is no global one-step offset between a
row's stamp and its contents: a 50 Hz `SIN()` source and an equivalent `kind=sinwave` block both
match $A\sin(2\pi f t)$ at the row's own stamp to full printed precision, and a `kind=pulsewave`
edge lands in the first row past the edge time, not the one after it.

**The one exception is `kind=phys2sig`.** A `phys2sig` column holds the circuit quantity as of
$t - \Delta t$, and `0` in the first row. The block graph is evaluated *before* the circuit is
solved — a block's output may decide a gate state or a source magnitude for the step about to be
solved — so a block that reads the circuit necessarily reads the previous step's solution. That
is this project's sampled-data coupling, documented as a deliberate choice in
[Signals](signals.md) and `book/dev-guide/src/block-graph-descriptor.md`, and it is the same
one-sample delay a real digital controller has.

So in the same row, a `phys2sig` column and the `V(...)` column it reads disagree by one step:

```text
V1 in 0 5
R1 in out 1k
R2 out 0 1k
PROBE1 kind=phys2sig node=out
```

```text
t,V(in),V(out),I(V1),PROBE1
0.001,5,2.5,-0.0025,0
0.002,5,2.5,-0.0025,2.5
0.003,5,2.5,-0.0025,2.5
```

On a settled circuit this only shows in the first row, which is why it is easy to miss. On a
switching or oscillating quantity it is a real one-sample phase shift in whatever loop is closed
through it. There is no flag that removes it; choose `--dt` small enough that one step's lag is
acceptable in that loop.

## `NaN` / missing columns

If a block's own output isn't resolved for a given point (a genuinely exceptional case — the
normal path resolves every declared block every step), that column reads `NaN` for that row
rather than the row being dropped or the column omitted. If the block-graph columns are missing
*entirely* (only `t`/circuit-unknown columns appear), the block graph wasn't evaluated at all for
this run — that only happens when the netlist declares neither an ideal switch nor a block, so there was
nothing for a block-graph step to resolve (see the module doc comment on
`crates/general-simulator-cli/src/main.rs`).

## `--format raw`'s variable-type metadata

A SPICE rawfile's `Variables:` section tags every column with a *type* (`time`/`voltage`/
`current`/...) in addition to its name — purely descriptive metadata, not something that changes
the actual value written. `V(node)`/`I(branch)` columns get `voltage`/`current` respectively; a
block's own output (a PID's control signal, a coordinate transform's `d`/`q` value, ...) isn't a
literal circuit voltage or current, but the SPICE rawfile format itself has no "generic signal"
type to give it, so it's tagged `voltage` too, as the broadest-compatibility fallback across the
readers this was validated against (see `raw_format.rs`'s own "Variable-type compatibility
caveat" doc section, and `tests/raw-output-python/README.md`'s "Compatibility caveats" for the
concrete reader behavior this was tested against). The *value* in that column is identical to
the same column's CSV value either way.

## Plotting it

The worked examples in an internal validation-experiment archive's experiment folders each
include a `plot_*.py` script reading this CSV shape directly with Python's own `csv` module or
`pandas.read_csv` (e.g. the plotting script from an internal three-phase PFC experiment) — a
concrete starting point rather than repeating a general Python/matplotlib tutorial here. A
`--format raw` file plots the same way through any SPICE-rawfile-aware tool instead (`spicelib`'s
own `RawRead.get_wave(name)`, or PySpice's `Spice.Xyce.RawFile.RawFile`, are the two this feature
was cross-validated against — see `tests/raw-output-python/`).

## Source material this was adapted from
- `crates/general-simulator-cli/src/main.rs`'s CSV-writing code and its comments on the
  `block_names`/extra-output-column logic.
- `crates/general-simulator-cli/src/raw_format.rs`'s module doc comment.
- Any internal validation experiment's `plot_*.py` as a concrete worked example of
  parsing the CSV (e.g. the plotting script from an internal three-phase PFC experiment).
