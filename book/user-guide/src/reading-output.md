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
