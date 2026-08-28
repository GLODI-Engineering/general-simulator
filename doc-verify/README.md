# doc-verify

Committed, reviewable proof that every `## Example` and `## Errors` claim in the
[Component Reference](../book/user-guide/src/component-reference.md) — generated from doc
comments on `general-mna`'s `BlockKind`/`GateBinding` — was actually run against the real
`general-simulator` CLI, not just plausible-looking prose. See
`../.claude/skills/write-component-doc/SKILL.md` before adding or editing an entry: **testing
before writing documentation is mandatory practice here, not optional.**

## Layout

One folder per component, named after its `kind=` token (`pid/`, `cscript/`, `pyblock/`,
`pyfunc/`, ...). Each folder has:

- `README.md` — what's tested and why, and how to run it.
- Commented `.cir` netlists (one per `## Example`/`## Errors` claim).
- Any `.c`/`.py` fixture an example depends on.
- `test_<kind>.py` — real unit tests (see `_lib.py` below), one function per documented
  behavior, runnable standalone or via `pytest`.

`_lib.py` is the shared harness every `test_<kind>.py` imports — subprocesses the real CLI
binary, asserts success/failure, and parses its CSV output. Don't hand-roll subprocess calls
per component; extend `_lib.py` if a component needs something it doesn't already provide.

## Running everything

```bash
cargo build --release -p general-simulator-cli --features python   # from the repo root, once
python3 -m pytest doc-verify -v          # if pytest is available, or:
for d in doc-verify/*/; do [ -f "$d"/test_*.py ] && python3 "$d"/test_*.py; done
```

## What's committed vs. gitignored

Everything here is real, committed source — `.cir`/`.c`/`.py`/`.md` files are reviewed like any
other change. Only what gets *compiled from* that source (`.so`/`.dylib`/`.dll`,
`__pycache__/`, `.pyc`) is gitignored — see the repo's own `.gitignore`.
