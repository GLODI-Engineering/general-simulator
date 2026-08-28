# doc-verify/pwm

Verification fixtures for the `PWM Modulator 1` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::Pwm`). Run `test_pwm.py` before editing that
doc comment.

## Files

- `example.cir` — the doc comment's own `## Example`: 10 kHz, 30% duty, no dead time. Checked
  that `PWM1 + PWM1_comp == 1` at every sampled instant across a full period.
- `error_bad_outputs_count.cir` — `outputs=` with 3 entries instead of 2, rejected at parse
  time.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/pwm/test_pwm.py
```
