# doc-verify/vco

Verification fixtures for the `VCO` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::Vco`). Run `test_vco.py` before editing that
doc comment.

## Files

- `example.cir` — the doc comment's own `## Example`: a fixed 10 Hz oscillator, checked to
  advance its `[0,1)` ramp by exactly `dt * freq` each step, not stay constant.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/vco/test_vco.py
```
