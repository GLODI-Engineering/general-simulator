# doc-verify/cscript

Verification fixtures for the `CScript` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::CScript`). Run `test_cscript.py` before editing
that doc comment.

## Files

- `gain.c` — a two-function `.c` fixture (`cscript_start`/`cscript_output` only, no
  `cscript_clone`) that doubles its single input. Compiled fresh by `test_cscript.py`, not
  committed as a binary (`.so` is gitignored — see the repo's own `.gitignore`).
- `example.cir` — the doc comment's own `## Example` netlist, using `gain.so` compiled straight
  into this directory.
- `example_quoted_path.cir` — the same netlist, but `lib=` points into the committed
  `a lib dir/` (a real, portable, space-containing directory name — not dependent on the
  checkout's own path happening to contain a space) to prove the `"double-quoting"` rule for a
  path containing whitespace.
- `error_adaptive_needs_clone.cir` — the same `gain.c` (no `cscript_clone`) run under adaptive
  stepping (no `--dt`), to reproduce `CScriptRequiresCloneForAdaptiveStep`.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/cscript/test_cscript.py
```
