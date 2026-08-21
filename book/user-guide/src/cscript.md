# The CScript escape hatch

*(Skeleton — outline below; not yet written.)*

## What goes here
- When to reach for `kind=cscript` instead of composing existing blocks: genuinely stateful
  behavior none of `continuous-blocks`'s own blocks cover.
- The C-side contract at a summary level (`cscript_start`/`cscript_output`/optionally
  `cscript_free`/`cscript_clone`), with a full minimal example `.c` file — link to the fixture
  files used in the crate's own tests rather than inventing a new example.
- `outputs=` (multi-output) and `ts=`/`freq=` (fixed sample-rate, zero-order hold) fields.
- **Adaptive step-size requires `cscript_clone`** — call this out prominently, with the exact
  error message a user will see if it's missing, so it's searchable.
- A safety note: this is the one place arbitrary native code loads into the process; say so
  plainly.

## Source material to adapt from
- `crates/cscript-ffi/src/lib.rs` module doc comment — the full C-side contract already lives
  here.
- `crates/elspice-pwl-cli/tests/fixtures/cscript_gain.c`/`cscript_counter.c` as ready-made
  minimal examples.
