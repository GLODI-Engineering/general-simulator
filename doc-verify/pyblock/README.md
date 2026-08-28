# doc-verify/pyblock

Verification fixtures for the `PyBlock` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::PyBlock`). Run `test_pyblock.py` before editing
that doc comment.

## Files

- `gain.py` — a two-function fixture (`start`/`output` only) that doubles its single input.
- `example.cir` — the doc comment's own `## Example` netlist.
- `a lib dir/gain.py` (committed, real space in the directory name — portable across clones,
  not dependent on the checkout's own path) + `example_quoted_path.cir` — proves `path=` must
  be `"double-quoted"` when it contains whitespace.
- `xc_decay.py` + `xc_example.cir` — exercises the `xc_count>0` continuous-state contract
  (`derivative`/`output_xc` instead of `output`): `dxc/dt = 1 - xc`, starting at rest, checked
  against its analytic solution `xc(t) = 1 - exp(-t)`.

## Running

Requires a `--features python` build (`kind=pyblock` needs a discoverable Python/`libpython` at
build/link time — see the doc entry's own `## Errors`):

```bash
cargo build --release -p general-simulator-cli --features python   # from the repo root, once
python3 doc-verify/pyblock/test_pyblock.py
```

## Not covered by the automated test

`PythonSupportNotCompiledIn` (the error a `--features python`-*less* build produces) was
verified manually this session, but isn't re-verified by `test_pyblock.py` on every run — doing
so would need a second binary built *without* the feature, alongside the one this folder's
other tests need built *with* it, which isn't worth the build-orchestration complexity for one
config-dependent error message. If you touch that error path, re-verify it manually:
`cargo build --release -p general-simulator-cli` (no `--features python`), then run
`example.cir` against that binary and confirm the error text still matches.
