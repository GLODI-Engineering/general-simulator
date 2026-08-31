# doc-verify/octfunc

Verification fixtures for the `OctFunc` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::OctFunc`). Run `test_octfunc.py` before editing
that doc comment.

## Files

- `add_one.m` — a one-function fixture (`add_one`) called as `add_one(x)`.
- `example.cir` — the doc comment's own `## Example` netlist.
- `a lib dir/add_one.m` (committed, real space in the directory name) + `example_quoted_path.cir`
  — proves `path=` must be `"double-quoted"` when it contains whitespace.
- `subtract.m` + `example_two_inputs.cir` — proves `inputs=A,B` calls `subtract(A, B)` as two
  genuinely distinct positional arguments, not a bundled array (`A=10, B=3` gives `7`, not `-7`,
  which would indicate the arguments were swapped or misordered).
- `error_ts_variable.cir` — `ts=variable`, rejected at parse time for `kind=octfunc` (the
  `## Errors` claim).

## Running

Requires `octave-cli` on `PATH` at run time (unlike `doc-verify/pyfunc`/`doc-verify/pyblock`, no
special `--features python` *build* is needed -- `kind=octfunc` builds and runs unconditionally,
see `octave_ffi`'s own module doc comment):

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once (no feature flag)
python3 doc-verify/octfunc/test_octfunc.py
```

## Not covered by the automated test

`octave_ffi::OctaveError::NotFound` (the error surfaced when `octave-cli` isn't on `PATH` at
all) and `OctaveError::ProcessExited` (the shared session dying mid-run) are both exercised
directly against a real `octave-cli` process in `crates/octave-ffi/tests/session.rs`
(`missing_octave_cli_reports_not_found`, `process_dies_mid_run_reports_clean_error_instead_of_hanging`)
— not re-verified here through the full CLI, since reproducing "PATH doesn't contain
`octave-cli`" or "kill the process mid-run" through a subprocess-of-a-subprocess integration
test would add real orchestration complexity for no additional coverage beyond what the crate's
own test suite already proves directly against the same `OctaveSession` type `dae-runtime`
actually uses.
