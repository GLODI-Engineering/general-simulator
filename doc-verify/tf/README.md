# doc-verify/tf

Verification fixtures for the `Transfer Function` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::TransferFunction`). Run `test_tf.py` before
editing that doc comment.

## Files

- `example.cir` — the doc comment's own `## Example`: $1/(s+1)$, the same first-order
  low-pass as `doc-verify/statespace`'s own example, checked to settle to the same value.
- `error_empty_denominator.cir` — `den=[]`, rejected at parse time.
- `error_improper.cir` — `num` with more coefficients than `den` (an improper transfer
  function), rejected at parse time.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/tf/test_tf.py
```
