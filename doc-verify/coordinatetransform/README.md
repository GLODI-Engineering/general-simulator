# doc-verify/coordinatetransform

Verification fixtures for the `Coordinate Transform (Clarke/Park)` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::CoordinateTransform`). Run
`test_coordinatetransform.py` before editing that doc comment.

## Files

- `example.cir` — the doc comment's own `## Example`: a balanced three-phase set through
  `clarke`, checked to give exactly `alpha=1, beta=0` for a peak on phase A.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/coordinatetransform/test_coordinatetransform.py
```
