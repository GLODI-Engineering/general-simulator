# doc-verify/discretetf

Verification fixtures for the `Discrete Transfer Function` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::DiscreteTransferFunction`). Run
`test_discretetf.py` before editing that doc comment.

## Files

- `example.cir` — the doc comment's own `## Example`: the exact $z$-domain realization of
  `doc-verify/discretestatespace`'s own example ($0.1/(z-1)$), checked to reach the same value.
- `y0_example.cir` — `y0=3` with the matching input ($H(1) = 1.5$, $u = 2$): a fixed point, exactly $3$ at every sample.
- `ic_example.cir` — `ic=[8]` on $1/(z-0.5)$ with no input: the output reads $4, 2, 1$.
- `error_ic_and_y0.cir` — `ic=` and `y0=` together, rejected at parse time.
- `error_y0_zero_dc_numerator.cir` — `y0=` on a transfer function with a zero at $z = 1$, rejected at parse time.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/discretetf/test_discretetf.py
```
