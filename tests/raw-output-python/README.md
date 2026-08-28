# raw-output-python

Committed, reviewable proof that `general-simulator --format raw` (see
`crates/general-simulator-cli/src/raw_format.rs`) produces a genuine SPICE rawfile, not just a
file that satisfies this crate's own writer -- two independent, real Python readers from the
SPICE tooling ecosystem parse a file this CLI actually wrote and the parsed values are checked
against the same run's `--format csv` output. This isn't a `kind=` netlist component (see
`doc-verify/README.md` for that framing), so it lives here instead of under `doc-verify/`.

## What's tested

`test_raw_output.py`:

- **PySpice** (`PySpice.Spice.Xyce.RawFile.RawFile`) reads a transient run's rawfile and its
  parsed variable names/values are asserted to match the same run's CSV. PySpice's `Xyce`
  submodule is used deliberately over its `NgSpice` one: the `NgSpice` reader expects the
  `Circuit:`/`Doing analysis at TEMP...` preamble lines that only appear in ngspice's own
  interactive console dump, not in a standalone `.raw` file written by `-r`/`write`, which is
  the shape this CLI's own rawfile has (see the ngspice manual's own worked rawfile example,
  §12.13).
- **spicelib** (`spicelib.RawRead`), a second, independent implementation, reads both a transient
  and a DC run's rawfile the same way. `spicelib` additionally validates against a whitelist of
  known SPICE dialects (`ngspice`/`xyce`/`qspice`, and one more well-known SPICE-family
  variant) rather than parsing unconditionally;
  since this CLI isn't literally any of those four tools, the tests pass `dialect="ngspice"`
  explicitly (the closest real match: double-precision values throughout a non-AC plot, the same
  convention this writer follows).

Both readers parsing the same file, and neither needing anything beyond an explicit dialect hint
to do so, is the actual cross-validation -- a bug that happened to satisfy one library's own
tolerant parsing wouldn't necessarily satisfy the other's.

## Setup

```bash
cd tests/raw-output-python
python3 -m venv .venv          # PEP 668 "externally-managed" blocks a system-wide pip install
.venv/bin/pip install -r requirements.txt
cargo build --release -p general-simulator-cli    # from the repo root
```

## Running

```bash
tests/raw-output-python/.venv/bin/python tests/raw-output-python/test_raw_output.py
# or, if pytest is installed in the venv:
tests/raw-output-python/.venv/bin/python -m pytest tests/raw-output-python -v
```

## Compatibility caveats found while validating

- **PySpice 1.5's own rawfile reader can't run under numpy 2.x as installed.**
  `PySpice.Spice.RawFile.RawFileAbc._read_variable_data` calls
  `np.fromstring(raw_data, dtype='f8')` in *binary* mode, which numpy 2.0 removed outright
  ("The binary mode of fromstring is removed, use frombuffer instead"). `requirements.txt`
  pins `numpy<2` to work around this -- a real PySpice/numpy compatibility issue discovered
  during this validation, unrelated to this crate's own writer.
- **`spicelib.RawRead` requires an explicit `dialect=` for a file whose `Command:` header field
  it doesn't recognize.** `general-simulator` doesn't emit a `Command:` line at all (there is no
  real ngspice/Xyce/qspice (or other SPICE-family tool's own) invocation behind it), so
  `spicelib`'s own dialect
  auto-detection has nothing to key off of and raises unless the caller passes
  `dialect="ngspice"` (or another of its four known values) directly -- see `_lib.py`'s
  `run_csv_and_raw` and the two `spicelib`-based tests in `test_raw_output.py`.
- **PySpice's `NgSpice.RawFile.RawFile` (as opposed to its `Xyce` counterpart) is the wrong
  reader for a standalone `.raw` file.** It expects a `Circuit: <name>` line and a
  `Doing analysis at TEMP = ...` line before `Title:`, which only appear when parsing ngspice's
  own interactive console transcript (see the ngspice manual's §12.13 example), not in the
  `.raw` file ngspice itself writes with `-r`/`write`. `Xyce.RawFile.RawFile` reads a bare
  `Title:`-first file directly and is the one used here.

See `crates/general-simulator-cli/src/raw_format.rs`'s own module doc comment for the exact byte
layout this writer produces, and `book/user-guide/src/reading-output.md` for the user-facing
data-mapping guarantee (raw output is the same data as CSV, just serialized differently).
