# doc-verify/pwc

Verification fixtures for the `Pwc` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::Pwc`). Run `test_pwc.py` before editing that doc
comment — see the `write-component-doc` skill's "Testing Examples and Errors against a real
run" section for why this is mandatory, not optional.

## Files

- `example.cir` — the doc comment's own `## Example` netlist, verbatim: a two-point step test
  (piecewise-constant hold-until-next-point), plus the `repeat=true` periodic form — with the
  three-point shape (low, high, low) a two-point list wrapped into `[first, last)` would never
  reach, so the "high" segment would have zero width.
- `error_bad_row.cir` — `points=[[0,0],[1]]`, a row that is not a 2-element `[x,y]` pair (the
  `Errors` section's pair claim). Expected to fail with
  `each entry must be a 2-element '[x,y]' list (got '[1.0]')`.
- `error_not_a_list.cir` — `points=abc` (the `Errors` section's Python-list claim). Expected
  to fail with `field 'points' must be a Python-style list, e.g. '[1,2,3]' or '[[1,2],[3,4]]'`.
- `out_of_order_repeat_case.cir` — two `Parameters` claims at once: `points=` is sorted
  ascending by `x` at parse time (newest-point-first behaves identically), and `repeat=` only
  matches the exact string `true` (`True` is not it, so the value holds flat past the end).

A missing `points=` entirely is the generic `missing field 'points'` error shared by every
`kind=` block (documented once on `BlockInstance`), so it has no dedicated fixture here.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/pwc/test_pwc.py
```
