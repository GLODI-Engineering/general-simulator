# doc-verify/pwl

Verification fixtures for the `Pwl` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::Pwl`). Run `test_pwl.py` before editing that doc
comment — see the `write-component-doc` skill's "Testing Examples and Errors against a real
run" section for why this is mandatory, not optional.

## Files

- `example.cir` — the doc comment's own `## Example` netlist, verbatim: linear interpolation
  with end-holds (a ramp), plus `repeat=true` wrapping `t` into `[first, last)` (a periodic
  triangle — the piecewise-linear periodic waveform the electrical-domain `PWL` source cannot
  express).
- `error_bad_row.cir` — `points=[[0,0],[1]]`, a row that is not a 2-element `[x,y]` pair (the
  `Errors` section's pair claim). Expected to fail with
  `each entry must be a 2-element '[x,y]' list (got '[1.0]')`.

`points=` shares the rest of its parser with `kind=pwc`: the Python-list requirement
(`field 'points' must be a Python-style list, ...`) is verified in `doc-verify/pwc/`, and
sorting at parse time / `repeat=`'s exact-string match are covered there and in
`doc-verify/table/out_of_order.cir`. A missing `points=` entirely is the generic
`missing field 'points'` error shared by every `kind=` block (documented once on
`BlockInstance`), so it has no dedicated fixture here.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/pwl/test_pwl.py
```
