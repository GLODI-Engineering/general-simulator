# doc-verify/sig2current

Verification fixtures for the `Sig2Current` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::Sig2Current`). Run `test_sig2current.py` before
editing that doc comment.

## Files

- `example.cir` — the doc comment's own `## Example`: a block-driven current source,
  `abs(V(a)) == CMD * R1` (Ohm's law) confirming the current magnitude is correctly driven.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/sig2current/test_sig2current.py
```
