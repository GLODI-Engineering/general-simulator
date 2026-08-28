# doc-verify/saturation

Verification fixtures for the `Saturation` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::Saturation`). Run `test_saturation.py` before
editing that doc comment — see the `write-component-doc` skill's "Testing Examples and Errors
against a real run" section for why this is mandatory, not optional.

## Files

- `example.cir` — the doc comment's own `## Example` netlist, verbatim: the `time` ramp
  through `limit=1`, showing pass-through while `|x| <= 1` and a hard hold at `1` beyond it.
- `error_negative_limit.cir` — `limit=-1` (the `Errors` section's negative-limit claim).
  Expected to *panic* (not a clean error — parse time never validates `limit`): stderr contains
  `limit must be nonnegative`, exit code nonzero.

A missing `limit=`/`in=` or a non-numeric `limit=` is the generic
`missing field '<key>'`/`field '<key>' is not a number` error shared by every `kind=` block
(documented once on `BlockInstance`), so it has no dedicated fixture here.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/saturation/test_saturation.py
```
