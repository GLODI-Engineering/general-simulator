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
- `ic_example.cir` — `ic=0.75` at 1 Hz, 50 % duty: the main output reads $0, 0, 1, 1, 1$.
- `error_ic_out_of_range.cir` — `ic=` outside $[0, 1)$, rejected at parse time.
- `phase_lead.cir` — the `phase` input is a **lead**: at 1 Hz, 50 % duty, $\Delta t = 0.125$ s,
  the block commanded `phase=0.25` rises at $t = 0.75$ s, the `phase=0` block at $t = 1.0$ s —
  a quarter period *earlier*, not later.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/pspwm/test_pspwm.py
```
