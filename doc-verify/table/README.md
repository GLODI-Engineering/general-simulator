# doc-verify/table

Verification fixtures for the `Table` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::Table`). Run `test_table.py` before editing that
doc comment — see the `write-component-doc` skill's "Testing Examples and Errors against a
real run" section for why this is mandatory, not optional.

## Files

- `example.cir` — the doc comment's own `## Example` netlist, verbatim: linear interpolation
  through `[[0,0],[1,10],[2,10],[3,0]]` including a flat segment, plus clamping (not
  extrapolating) outside the breakpoint range.
- `error_bad_row.cir` — `points=[[0,0],[1]]`, a row that is not a 2-element `[x,y]` pair (the
  `Errors` section's pair claim). Expected to fail with
  `each entry must be a 2-element '[x,y]' list (got '[1.0]')`.
- `error_empty.cir` — `points=[]` (the `Errors` section's empty-table claim). Expected to
  *panic* (not a clean error — parse time never rejects an empty list): stderr contains
  `table needs at least one point`, exit code nonzero.
- `out_of_order.cir` — a `Parameters` claim: `points=` is sorted ascending by `x` at parse
  time, so declaration order does not matter.

`points=` shares the rest of its parser with `kind=pwc`/`kind=pwl`: the Python-list
requirement (`field 'points' must be a Python-style list, ...`) is verified in
`doc-verify/pwc/`. A missing `points=`/`in=` is the generic `missing field '<key>'` error
shared by every `kind=` block (documented once on `BlockInstance`), so it has no dedicated
fixture here.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/table/test_table.py
```
