# doc-verify/pmsm

Verification fixtures for the `PMSM` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::Pmsm`). Run `test_pmsm.py` before editing that
doc comment.

## Files

- `example.cir` — the doc comment's own `## Example`: zero applied voltage/load, checked to
  stay exactly at rest (`id = iq = omega_m = theta_e = 0`) for the whole run.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/pmsm/test_pmsm.py
```
