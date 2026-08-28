# doc-verify/time

Verification fixtures for the `Time` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::Time`). Run `test_time.py` before editing that
doc comment — see the `write-component-doc` skill's "Testing Examples and Errors against a
real run" section for why this is mandatory, not optional.

## Files

- `example.cir` — the doc comment's own `## Example` netlist, verbatim: the standard
  `sin(2*pi*f*t)` recipe (`time -> gain(k=2*pi*1000) -> sin`), proving the Time block outputs
  the current step's own simulated time.

## Not covered by the automated test

`kind=time` takes no fields at all, so it has no component-specific error paths to fixture —
there is no `## Errors` section in this entry. (The generic `unknown device kind` error for a
misspelled kind is covered by `doc-verify/mathfn/error_unknown_kind.cir`.)

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/time/test_time.py
```
