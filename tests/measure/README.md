# measure

Committed, reviewable end-to-end proof that `kind=measure` netlist lines (see
`crates/general-simulator-cli/src/measure.rs` and `book/user-guide/src/measurements.md`) produce
correct results through the real CLI, for every measurement type
[`gs-waveform-measurements`](../../../gs-waveform-measurements) implements. This isn't a `kind=`
block-graph component (`general-mna` never even sees a `kind=measure` line, see `measure.rs`'s
own module doc comment), so it lives here rather than under `doc-verify/`, the same reasoning
`tests/raw-output-python/README.md` documents for its own feature.

Every test runs the actual built `general-simulator` binary as a subprocess against a committed
`.cir` fixture under `fixtures/` and parses its `stderr` (`kind=measure` results always print to
stderr, never stdout — see `measure.rs`'s `print_measurements` doc comment for why). No mocking,
no calling into Rust measurement code directly.

## What's tested, and why each expected value is trustworthy

Every fixture drives its signal(s) from an exact, closed-form source (`kind=pwl` two-point ramps,
`kind=sinwave`/`kind=pulsewave` — real SPICE `SIN()`/`PULSE()` sources) fed onto a node through an
ideal ­­`sig2phys domain=voltage`/V-source pair, so the measured node voltage is *exactly* that closed-form
function of time at every simulated instant — never an approximation from real circuit dynamics.
Every expected value below is a hand-derived closed-form number, not something read back from the
tool's own output (see each fixture's own header comment for the derivation):

| Test | Fixture | Measure types covered |
|---|---|---|
| `test_ramp.py` | `ramp.cir` | `max`, `min`, `max_at`, `min_at`, `pp`, `avg`, `rms`, `integ`, `deriv` (`at=` and `when=`), `find` (`at=` and `when=`), `when`, `trig_targ` |
| `test_square.py` | `square.cir` | `freq`, `on_time`, `off_time` |
| `test_sine.py` | `sine.cir` | `four` — pure sine, fixed-dt baseline: fundamental magnitude/phase against the analytic answer |
| `test_two_tone.py` | `two_tone.cir` | `four` — THD against a hand computation (a known second harmonic of known relative amplitude) |
| `test_sine_adaptive.py` | `sine_adaptive.cir` | `four` on a **genuinely non-uniform-timestep** trace — the entire reason this architecture (and `gs-waveform-measurements` itself) exists; asserts the trace really is non-uniform (not just "ran without `--dt`") *and* that `FOUR` still recovers the correct analytic answer on it |
| `test_err.py` | `err.cir` | `err1`, `err2`, `error` (`norm=l1`/`l2`/`infnorm`) |

`max`/`min`/`max_at`/`min_at`/`pp`/`avg`/`rms`/`integ`/`deriv`/`find`/`when`/`trig_targ`/`freq`/
`on_time`/`off_time`/`four`/`err1`/`err2`/`error` — every measure type
`gs-waveform-measurements` implements — is exercised here. `EQN` and the `FILE=` half of `ERROR`
are not implemented by `gs-waveform-measurements` itself (see its own `src/lib.rs` doc comment
and journal for why), so there is nothing to test for either.

## Setup

```bash
cargo build --release -p general-simulator-cli   # from the general-simulator repo root
# (a debug build also works -- _lib.py falls back to target/debug if target/release doesn't
# exist -- but release is faster for the finer-dt fixtures like sine.cir/two_tone.cir)
```

No Python packages beyond the standard library are needed (unlike `tests/raw-output-python`,
which needs PySpice/spicelib to parse a binary rawfile) — these tests only run the CLI and parse
its plain-text CSV/stderr output.

## Running

```bash
for f in tests/measure/test_*.py; do python3 "$f"; done
# or, if pytest is installed:
python3 -m pytest tests/measure -v
```

## A caveat found while writing these fixtures

The resolved transient trace's first row is `t = dt` (the first *completed* step), not `t = 0`
— there is no `t=0` row in the CSV/raw output at all (this predates `kind=measure`; it's how
`dae-runtime`'s own transient loop has always worked). A measurement whose window naturally
starts at the domain's own beginning (`min`/`min_at` on a monotonically increasing ramp, in
`ramp.cir`) therefore reports the value/time of the *first resolved sample*, not literally `t=0`
— `test_ramp.py`'s expected values account for this (`min = RAMP(dt)`, not `RAMP(0)`), rather
than papering over it with an oversized tolerance.
