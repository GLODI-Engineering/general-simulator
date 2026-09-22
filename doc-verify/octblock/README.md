# doc-verify/octblock

Verification fixtures for the `OctBlock` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::OctBlock`). Run `test_octblock.py` before editing
that doc comment.

## Files

- `accum/accumulate_start.m` + `accum/accumulate.m` — the doc comment's own `## Example`: a
  stateful accumulator (`out = state + in`, state advances by `in` every step). Two files for
  one function, per the file-per-function contract.
- `example.cir` — the doc comment's own `## Example` netlist.
- `xc/xc_charge_start.m` + `xc/xc_charge_derivative.m` + `xc/xc_charge_output_xc.m` +
  `xc_example.cir` — the `xc_count>0` contract, mirroring `doc-verify/pyblock/xc_example.cir`'s
  own precedent: a first-order charge, `dxc/dt = 1 - xc`, checked against its own analytic
  solution `xc(t) = 1 - exp(-t)`.
- `ticker/` + `ts_variable_example.cir` — `ts=variable` support (requires
  `ticker_next_sample_hit.m`, which always requests a fixed 0.002s interval).
- `missing_output/accumulate_start.m` + `error_missing_required_file.cir` — proves a required
  `<function>_*.m` file missing from `path`'s own directory (here, `accumulate.m` itself) is
  rejected at construction time with `OctBlockMissingRequiredFile`, before `octave-cli` is ever
  asked about it.
- `error_ts_variable_missing_next_sample_hit.cir` — `ts=variable` against `accum/` (which has no
  `accumulate_next_sample_hit.m`) is rejected the same way, with
  `OctBlockRequiresNextSampleHitForVariableSampleTime`.
- `error_adaptive_not_supported.cir` — run with no `--dt` (adaptive stepping): rejected at
  construction time with `OctBlockDoesNotSupportAdaptiveStep`, since this instance's own opaque
  state lives inside the one shared `octave-cli` session, which a rejected adaptive trial step
  cannot roll back (see that error's own doc comment in `dae_runtime::DaeError`).

- `checkpoint_example.cir` — backs the user guide's Octave chapter, "Checkpoint and resume" (and
  the checkpoint chapter's escape-hatch table): the unmodified `accum/` block run whole and
  split across `--checkpoint-out`/`--resume`, asserting the two CSVs are byte for byte
  identical — the state slot round-trips through Octave's own `save -binary`/`load` with no
  author change.

## Running

Requires `octave-cli` on `PATH` at run time (no special `--features python` *build* is needed —
`kind=octblock` builds and runs unconditionally, see `octave_ffi`'s own module doc comment):

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once (no feature flag)
python3 doc-verify/octblock/test_octblock.py
```

## Not covered by the automated test

Per-instance state isolation across two instances of the same function, the `_update.m`
present-vs-absent distinction, and error-then-recovery without state corruption are all
exercised directly against a real `octave-cli` process in
`crates/octave-ffi/tests/stateful_session.rs` — not re-verified here through the full CLI, for
the same reason `doc-verify/octfunc/README.md` gives for `OctaveError::NotFound`/
`ProcessExited`: the crate's own test suite already proves it directly against the same
`OctaveSession` type `dae-runtime` actually uses, without the added orchestration complexity of
reproducing it through a subprocess-of-a-subprocess integration test.
