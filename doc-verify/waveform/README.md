# doc-verify/waveform

Verification fixtures for the `Waveform` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::Waveform` — the `kind=sinwave`/`pulsewave`/
`expwave`/`sffmwave` sources). Run `test_waveform.py` before editing that doc comment — see
the `write-component-doc` skill's "Testing Examples and Errors against a real run" section for
why this is mandatory, not optional.

## Files

- `example.cir` — the doc comment's own `## Example` netlist, verbatim: all four sub-forms at
  once, each checked against its closed-form SPICE formula (SIN damped sinusoid, PULSE
  periodic trapezoid, EXP two-stage exponential, SFFM single-frequency FM with `mdi=0`).
- `error_missing_va.cir` — `sinwave` without its required `va` (the `Errors` section's
  required-field claim). Expected to fail with `missing field 'va'`.
- `error_bad_va.cir` — `va=abc` (the `Errors` section's not-a-number claim). Expected to fail
  with `field 'va' is not a number`.
- `error_malformed_optional.cir` — `td=abc` (the `Errors` section's silent-default claim).
  Expected to *succeed*, behaving exactly as if `td` were `0` (the default): optional fields
  fall back to their default on a failed parse rather than erroring.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/waveform/test_waveform.py
```
