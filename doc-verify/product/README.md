# doc-verify/product

Verification fixtures for the `Product` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::Product`). Run `test_product.py` before editing
that doc comment — see the `write-component-doc` skill's "Testing Examples and Errors against
a real run" section for why this is mandatory, not optional.

## Files

- `example.cir` — the doc comment's own `## Example` netlist, verbatim: three inputs multiplied,
  `P = 2 * 3 * 4 = 24`.
- `error_mixed.cir` — one Scalar and one Vector input (the `Errors` section's uniform-shape
  claim, shared with `Sum`). Expected to fail with
  `VectorSignalNotSupported { block: "BAD" }` at evaluation time.
- `error_length.cir` — Vector inputs of lengths 2 and 3 (the `Errors` section's common-length
  claim). Expected to fail with
  `VectorSignalSizeMismatch { block: "BAD", expected: 2, got: 3 }` at evaluation time.

A missing `inputs=` entirely is the generic `missing field 'inputs'` error shared by every
`kind=` block (documented once on `BlockInstance`), so it has no dedicated fixture here.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/product/test_product.py
```
