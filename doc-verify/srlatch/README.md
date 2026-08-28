# doc-verify/srlatch

Verification fixtures for the `SR Latch` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::SrLatch`). Run `test_srlatch.py` before editing
that doc comment.

## Files

- `example.cir` — the doc comment's own `## Example`: `FAULT` pulses to `1` then returns to
  `0` at `t=0.1`, `LATCH1` stays `1` afterward (no reset asserted).
- `error_bad_priority.cir` — `priority=bogus` (not `set`/`reset`), rejected at parse time.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/srlatch/test_srlatch.py
```
