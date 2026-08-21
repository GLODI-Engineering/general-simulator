# Command-line flags

*(Skeleton — outline below; not yet written.)*

## What goes here
- Full flag table: `<netlist>`, `--devices <file>`, `--mode {dc|transient}`, `--tfinal`,
  `--dt`, `--dt-max`/`--dt-min`/`--dt-init`/`--reltol`/`--abstol`, and the mutual-exclusivity
  rule between `--dt` and the `--dt-*`/`--reltol`/`--abstol` group.
- Exit codes / error message format (a one-line note; most detail belongs per-error in the
  gotchas/troubleshooting chapter instead).
- A note that `--mode dc` rejects block-driven gates (`gate=vco`/`dutyctrl`/`block`/`vcophase`)
  with a specific error, and why (no notion of a block's time-stepped state at a single
  operating point).

## Source material to adapt from
- `crates/elspice-pwl-cli/src/main.rs`'s `usage()` function and its argument-parsing code.
