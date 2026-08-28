# doc-verify/probe

Verification fixtures for the `Probe` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::Probe`). Run `test_probe.py` before editing that
doc comment.

## Files

- `example.cir` — the doc comment's own `## Example`: probing a resistor-divider node,
  `PROBE1` tracking `V(out)` exactly.
- `error_both_node_and_branch.cir` — `node=`/`branch=` both given, rejected at parse time.
- `error_neither.cir` — neither given, rejected at parse time.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/probe/test_probe.py
```
