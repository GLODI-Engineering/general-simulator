# doc-verify/statespace

Verification fixtures for the `StateSpace` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::StateSpace`). Run `test_statespace.py` before
editing that doc comment.

## Files

- `example.cir` — the doc comment's own `## Example`: a first-order low-pass
  ($\dot{x}=-x+u$), `SRC=1` settling toward `SS1=1`.
- `error_scalar_d_mimo.cir` — a bare scalar `d=0` on a genuinely MIMO (2-input, 1-output)
  system, rejected at parse time (the `## Errors` claim).
- `ic_example.cir` — `ic=[1,0]` on an undriven oscillator: the output is $\cos t$.
- `error_ic_wrong_length.cir` — `ic=` with the wrong number of entries, rejected at parse time.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/statespace/test_statespace.py
```
