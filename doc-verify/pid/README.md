# doc-verify/pid

Verification fixtures for the `PID` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::Pid`). Run `test_pid.py` before editing that
doc comment — see the `write-component-doc` skill's "Testing Examples and Errors against a
real run" section for why this is mandatory, not optional.

## Files

- `example.cir` — the doc comment's own `## Example` netlist, verbatim. `ERR` is a fixed zero
  error signal; with `kp=1 ki=0 kd=0`, `PID1`'s output must stay exactly `0` every step.
- `nonzero_error.cir` — proves `kp` is genuinely applied (not silently ignored): `ERR=0.5`,
  `kp=0.3`, clamp bounds wide enough that the clamp never engages, expected `PID1 = 0.15`.
- `error_nonpositive_n.cir` — `n=0` (the `Errors` section's `NonPositiveFilterCoefficient`
  claim). Expected to fail with `invalid PID (NonPositiveFilterCoefficient)`.
- `error_unpaired_dynamic_clamp.cir` — `clamp_lo_in` given without `clamp_hi_in` (the `Errors`
  section's clamp-pairing claim). Expected to fail with the `'clamp_lo_in'/'clamp_hi_in' must
  both be given together...` message.
- `ic_example.cir` — `ic=0.6` with zero error: the output holds $0.6$.
- `error_ic_without_integrator.cir` — `ic=` with `ki=0`, rejected at parse time.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/pid/test_pid.py
```
