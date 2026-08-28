# doc-verify/pyfunc

Verification fixtures for the `PyFunction` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::PyFunction`). Run `test_pyfunc.py` before
editing that doc comment.

## Files

- `add.py` — a one-function fixture (`add_one`) called as `add_one(*inputs)`.
- `example.cir` — the doc comment's own `## Example` netlist.
- `a lib dir/add.py` (committed, real space in the directory name) + `example_quoted_path.cir`
  — proves `path=` must be `"double-quoted"` when it contains whitespace.
- `subtract.py` + `example_two_inputs.cir` — proves `inputs=A,B` calls `subtract(A, B)` as two
  genuinely distinct positional arguments, not a bundled list (`A=10, B=3` gives `7`, not `-7`,
  which would indicate the arguments were swapped or misordered).
- `error_ts_variable.cir` — `ts=variable`, rejected at parse time for `kind=pyfunc` (the
  `## Errors` claim).

## Running

Requires a `--features python` build — same as `doc-verify/pyblock` (see its own README's
"Not covered by the automated test" for why `PythonSupportNotCompiledIn` isn't re-verified
here either, for the same reason):

```bash
cargo build --release -p general-simulator-cli --features python   # from the repo root, once
python3 doc-verify/pyfunc/test_pyfunc.py
```
