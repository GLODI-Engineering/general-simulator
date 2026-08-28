# doc-verify/discretetf

Verification fixtures for the `Discrete Transfer Function` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::DiscreteTransferFunction`). Run
`test_discretetf.py` before editing that doc comment.

## Files

- `example.cir` — the doc comment's own `## Example`: the exact $z$-domain realization of
  `doc-verify/discretestatespace`'s own example ($0.1/(z-1)$), checked to reach the same value.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/discretetf/test_discretetf.py
```
