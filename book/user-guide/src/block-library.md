# Block library overview

*(Skeleton — outline below; not yet written.)*

## What goes here
- A single table: every `kind=` value, one-line description, input count, output count, which
  detail chapter covers it.
- The organizing idea stated once, up front: "each block does one job and is wired to the
  others by the caller" — an error signal is a `sum` block's own output, a frequency-modulated
  PWM carrier is `pid -> gain -> vco`, never a single fused function — so a reader isn't
  surprised later that there's no monolithic "PID controller with built-in PWM" block.
- Pointer to the waveform-arithmetic function list (`cos`, `sin`, `atan2`, `if`, `limit`, ...)
  as its own short reference table rather than one block kind per function.

## Source material to adapt from
- `crates/continuous-blocks/src/lib.rs`'s module doc comment.
- `crates/elspice-pwl-cli/src/main.rs`'s module doc comment (the full `kind=` list with field
  signatures) — this chapter's table is essentially that list, reformatted.
