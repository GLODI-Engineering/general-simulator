# doc-verify/hysteresis

Verification fixtures for the `Hysteresis (Schmitt Trigger)` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::Hysteresis`). Run `test_hysteresis.py` before
editing that doc comment.

## Files

- `example.cir` — the doc comment's own `## Example`: a `[-1,1]` band fed a rising ramp, LOW
  until crossing `1`, then HIGH.
- `error_low_exceeds_high.cir` — `low > high`, rejected at parse time.
- `ic_example.cir` — `ic=1` with nothing driving a change: the output holds $1$.
- `error_ic_not_a_logic_level.cir` — `ic=` that is neither `0` nor `1`, rejected at parse time.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/hysteresis/test_hysteresis.py
```
