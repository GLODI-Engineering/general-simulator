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
- `ic_example.cir` — `ic=[0.5]` on $1/(s+1)$: the output follows $1 - 0.5\,e^{-t}$ from the first step.
- `y0_example.cir` — `y0=6` with the matching input ($G(0) = 1.5$, $u = 4$): an equilibrium, the output never moves.
- `error_ic_wrong_length.cir` — `ic=` with the wrong number of entries, rejected at parse time.
- `error_ic_and_y0.cir` — `ic=` and `y0=` together, rejected at parse time.
- `error_y0_zero_dc_numerator.cir` — `y0=` on a transfer function whose numerator vanishes at DC, rejected at parse time.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/tf/test_tf.py
```
