# doc-verify/gain

Verification fixtures for the `Gain` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::Gain`). Run `test_gain.py` before editing that
doc comment — see the `write-component-doc` skill's "Testing Examples and Errors against a
real run" section for why this is mandatory, not optional.

## Files

- `example.cir` — the doc comment's own `## Example` netlist, verbatim: the scalar form
  (`k=3`) and the matrix form (`k=[[1,0,0],[0,1,0]]`, a 2x3 row-major matrix times a length-3
  vector, outputting the matrix-vector product `[1,2]`).
- `error_flat_list.cir` — `k=[1,2,3]`, a flat list (the `Errors` section's flat-list claim).
  Expected to fail with the `must be a scalar ... or a matrix ... got a flat list` message.
- `error_not_number.cir` — `k=abc` (the `Errors` section's not-a-number claim). Expected to
  fail with `field 'k' is not a number`.
- `error_scalar_input.cir` — a matrix `k` fed by a Scalar input (the `Errors` section's
  matrix-needs-vector claim). Expected to fail with
  `VectorSignalNotSupported { block: "A" }` at evaluation time.
- `error_size_mismatch.cir` — a 2-column matrix `k` fed by a length-3 vector (the `Errors`
  section's column-count claim). Expected to fail with
  `VectorSignalSizeMismatch { block: "A", expected: 2, got: 3 }` at evaluation time.

A missing `in=`/`k=` entirely is the generic `missing field '<key>'` error shared by every
`kind=` block (documented once on `BlockInstance`), so it has no dedicated fixture here.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/gain/test_gain.py
```
