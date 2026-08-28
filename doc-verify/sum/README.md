# doc-verify/sum

Verification fixtures for the `Sum` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::Sum`). Run `test_sum.py` before editing that doc
comment — see the `write-component-doc` skill's "Testing Examples and Errors against a real
run" section for why this is mandatory, not optional.

## Files

- `example.cir` — the doc comment's own `## Example` netlist, verbatim: a scalar error
  junction (`signs=1,-1`) and an elementwise vector sum of two length-2 vectors.
- `error_count.cir` — 2 `inputs=` entries but 1 `signs=` entry (the `Errors` section's
  one-sign-per-input claim). Expected to fail with
  `'inputs' has 2 entries but 'signs' has 1 (need one sign per input)`.
- `error_bad_sign.cir` — `signs=1,x` (the `Errors` section's not-a-number claim). Expected to
  fail with `field 'signs' entry 'x' is not a number`.
- `error_mixed.cir` — one Scalar and one Vector input (the `Errors` section's uniform-shape
  claim). Expected to fail with `VectorSignalNotSupported { block: "BAD" }` at evaluation time.
- `error_length.cir` — Vector inputs of lengths 2 and 3 (the `Errors` section's common-length
  claim). Expected to fail with
  `VectorSignalSizeMismatch { block: "BAD", expected: 2, got: 3 }` at evaluation time.

Missing `inputs=`/`signs=` entirely is the generic `missing field '<key>'` error shared by
every `kind=` block (documented once on `BlockInstance`), so it has no dedicated fixture here.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/sum/test_sum.py
```
