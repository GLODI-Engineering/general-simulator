# doc-verify/logicgate

Verification fixtures for the `Logic Gate` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::LogicGate`). Run `test_logicgate.py` before
editing that doc comment.

## Files

- `example.cir` — the doc comment's own `## Example`: `1 and 0 = 0`.
- `error_too_few_inputs.cir` — `and` with only 1 `inputs=` entry, rejected at parse time.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/logicgate/test_logicgate.py
```
