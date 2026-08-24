# Reading the CSV output

*(Skeleton — outline below; not yet written.)*

## What goes here
- Column layout: `t`, then one column per circuit unknown (`V(node)`/`I(branch)`), then one
  column per declared block (by name), including any block's *extra* named outputs
  (`CScript`/`CoordinateTransform`/`Pmsm` with `outputs=`).
- How to plot it (a short, tool-agnostic note — the worked examples all use Python/matplotlib
  via a `plot_*.py` script; link there rather than repeating).
- A note on `NaN`/missing columns: what it means if a block name doesn't appear in a row
  (block graph wasn't evaluated at all — no MOSFET declared — see CLI reference).

## Source material to adapt from
- `crates/general-simulator-cli/src/main.rs`'s CSV-writing code and its comments on the
  `block_names`/extra-output-column logic.
- Any `internal-archive` experiment's `plot_*.py` as a concrete worked example of
  parsing the CSV (e.g. `experiments/elspice-pwl-pfc-three-phase-vsc/code/plot_stage1.py`).
