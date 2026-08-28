# doc-verify/const

Verification fixtures for the `Const` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::Const`). Run `test_const.py` before editing that
doc comment — see the `write-component-doc` skill's "Testing Examples and Errors against a
real run" section for why this is mandatory, not optional.

## Files

- `example.cir` — the doc comment's own `## Example` netlist, verbatim: the scalar form
  (`value=5`) and the vector form (`value=[1,2,3]`, a Python list literal), both time-invariant.
- `error_not_number.cir` — scalar `value=abc` (the `Errors` section's not-a-number claim).
  Expected to fail with `field 'value' is not a number`.
- `error_bad_entry.cir` — vector `value=[1,x,3]` (the `Errors` section's not-a-number entry
  claim). Expected to fail with `field 'value' entry 'x' is not a number`.
- `error_nan.cir` — vector `value=[1,nan,3]` (the `Errors` section's non-finite-entry claim).
  Expected to fail with `field 'value' entry 'nan' must be a finite number (got NaN)`.

A missing `value=` entirely is the generic `missing field 'value'` error shared by every
`kind=` block (documented once on `BlockInstance`), so it has no dedicated fixture here.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/const/test_const.py
```
