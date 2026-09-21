# doc-verify/flipflop

Verification fixtures for the `Flip-Flop` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::FlipFlop`). Run `test_flipflop.py` before
editing that doc comment.

## Files

- `example.cir` — the doc comment's own `## Example`: a `kind=dff` only updates `Q1` to `D`'s
  value at `CLK`'s rising edge (`t=5e-3`), staying `0` before it.
- `ic_example.cir` — `ic=1` with nothing driving a change: the output holds $1$.
- `error_ic_not_a_logic_level.cir` — `ic=` that is neither `0` nor `1`, rejected at parse time.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/flipflop/test_flipflop.py
```
