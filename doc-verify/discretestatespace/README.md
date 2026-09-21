# doc-verify/discretestatespace

Verification fixtures for the `Discrete State-Space` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::DiscreteStateSpace`). Run
`test_discretestatespace.py` before editing that doc comment.

## Files

- `example.cir` — the doc comment's own `## Example`: a discrete integrator
  ($x[i+1]=x[i]+0.1u[i]$, `ts=0.1`), `DS1` reaching exactly `0.5` after 5 sample hits.
- `ic_example.cir` — `ic=8` on $x[k+1] = 0.5\,x[k]$: the output reads $4, 2, 1$.
- `error_ic_wrong_length.cir` — `ic=` with the wrong number of entries, rejected at parse time.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/discretestatespace/test_discretestatespace.py
```
