# doc-verify/mathfn

Verification fixtures for the three grouped waveform-arithmetic component reference entries
(`general-mna/src/block_graph.rs`, `BlockKind::MathFn1`/`MathFn2`/`MathFn3` — the `kind=cos`/
`sin`/`exp`/... `kind=atan2`/`hypot`/... and `kind=if`/`limit` functions, one entry per
variant per the `write-component-doc` skill's grouping rule). Run `test_mathfn.py` before
editing any of those doc comments — see the skill's "Testing Examples and Errors against a
real run" section for why this is mandatory, not optional.

## Files

- `example_fn1.cir` — the MathFn1 entry's own `## Example` netlist, verbatim: `sin`/`cos`/`exp`
  of a `time -> gain` ramp (the `sin(2*pi*f*t)` recipe) plus the elementwise-Vector rule
  (`sqrt` of `[1,4,9]`).
- `example_fn2.cir` — the MathFn2 entry's own `## Example` netlist, verbatim: `hypot(3,4)` and
  `anglewrap(3,4)` (the `atan2`-derived angle wrapped to `[0, 2*pi)`).
- `example_fn3.cir` — the MathFn3 entry's own `## Example` netlist, verbatim: `if` (a
  threshold select) and `limit` (clamping to the span of its two bounds).
- `broadcast.cir` — the MathFn2 entry's `Parameters` broadcast claim: a lone Scalar operand
  applies elementwise against a Vector operand (`hypot(4, [3,4,5])`).
- `error_unknown_kind.cir` — `kind=cosx` (the `Errors` section's unknown-name claim). Expected
  to fail with `unknown device kind 'cosx'`.
- `error_missing_in.cir` / `error_missing_in2.cir` / `error_missing_in3.cir` — the `Errors`
  sections' missing-argument claims (generic `missing field '<key>'` errors, one per arity).
- `error_size_mismatch.cir` — the MathFn2 entry's `Errors` common-length claim: Vector operands
  of lengths 2 and 3. Expected to fail with
  `VectorSignalSizeMismatch { block: "A", expected: 2, got: 3 }` at evaluation time.

(MathFn3 shares MathFn2's `common_vector_len` broadcast/mismatch contract; it is verified here
via MathFn2's fixtures rather than duplicated per entry.)

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/mathfn/test_mathfn.py
```
