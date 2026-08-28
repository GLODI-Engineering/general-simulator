# doc-verify/discretepid

Verification fixtures for the `Discrete PID Controller` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::DiscretePid`). Run `test_discretepid.py` before
editing that doc comment.

## Files

- `example.cir` — the doc comment's own `## Example`: a discrete PI (`kd=0`), a fixed error
  signal, stepping `0.5 -> 0.55 -> 0.575 -> 0.575 -> 0.6` at each `ts=0.1` sample hit.
- `error_unknown_method.cir` — `integration_method=euler` (not one of `forward`/`backward`/
  `trapezoidal`), rejected at parse time (the `## Errors` claim).

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/discretepid/test_discretepid.py
```
