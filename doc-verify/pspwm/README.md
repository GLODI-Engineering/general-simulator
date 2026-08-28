# doc-verify/pspwm

Verification fixtures for the `PWM Modulator 2 (Phase-Shift PWM)` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::PhaseShiftPwm`). Run `test_pspwm.py` before
editing that doc comment.

## Files

- `example.cir` — the doc comment's own `## Example`: 10 kHz, zero phase shift, 40% duty.
  Checked that `PSPWM1 + PSPWM1_comp == 1` at every sampled instant.
- `error_bad_inputs_count.cir` — `inputs=` with 2 entries instead of the required 3
  (`freq,phase,duty`), rejected at parse time.
- `error_fmin_exceeds_fmax.cir` — `f_min > f_max`, rejected at parse time.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/pspwm/test_pspwm.py
```
