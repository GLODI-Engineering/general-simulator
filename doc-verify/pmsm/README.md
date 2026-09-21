# doc-verify/pmsm

Verification fixtures for the `PMSM` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::Pmsm`). Run `test_pmsm.py` before editing that
doc comment.

## Files

- `example.cir` — the doc comment's own `## Example`: zero applied voltage/load, checked to
  stay exactly at rest (`id = iq = omega_m = theta_e = 0`) for the whole run.
- `error_ic_wrong_length.cir` — `ic=` with the wrong number of entries, rejected at parse time.
- `ic_example.cir` — `ic=[0,0,100,0.5]` on a torque-free machine: it coasts at $100$ rad/s, $\theta_e = 0.5 + 400\,t$.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/pmsm/test_pmsm.py
```
