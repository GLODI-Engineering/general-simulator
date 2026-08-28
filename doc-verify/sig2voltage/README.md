# doc-verify/sig2voltage

Verification fixtures for the `Sig2Voltage` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::Sig2Voltage`). Run `test_sig2voltage.py` before
editing that doc comment.

## Files

- `example.cir` — the doc comment's own `## Example`: a block-driven voltage source, `V(a)`
  tracking `CMD` exactly.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/sig2voltage/test_sig2voltage.py
```
