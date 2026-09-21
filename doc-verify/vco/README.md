# doc-verify/vco

Verification fixtures for the `VCO` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::Vco`). Run `test_vco.py` before editing that
doc comment.

## Files

- `example.cir` — the doc comment's own `## Example`: a fixed 10 Hz oscillator, checked to
  advance its `[0,1)` ramp by exactly `dt * freq` each step, not stay constant.
- `ic_example.cir` — `ic=0.25` at 1 Hz: the phase reads $0.25 + t \pmod 1$.
- `error_ic_out_of_range.cir` — `ic=` outside $[0, 1)$, rejected at parse time.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/vco/test_vco.py
```
