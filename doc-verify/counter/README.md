# doc-verify/counter

Verification fixtures for the `Counter` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::Counter`). Run `test_counter.py` before editing
that doc comment.

## Files

- `example.cir` — the doc comment's own `## Example`: a free-running up counter clocked by a
  repeating pulse train, incrementing by exactly `1` at each detected clock rising edge.
- `error_bad_modulus.cir` — `modulus=abc` (not a non-negative integer), rejected at parse time.
- `ic_example.cir` — `ic=7` with no clock edge: the count holds $7$.
- `error_ic_not_an_integer.cir` — non-integer `ic=`, rejected at parse time.
- `error_ic_outside_modulus.cir` — `ic=` outside $[0, \text{modulus})$, rejected at parse time.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/counter/test_counter.py
```
