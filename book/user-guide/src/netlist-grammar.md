# Grammar overview

*(Skeleton — outline below; not yet written.)*

## What goes here
- The core convention: every device/block declaration is an ordinary SPICE comment line
  (`*`-prefixed) with a `name kind=<kind> field=value ...` payload; a line without `kind=` is
  left alone (including a genuine comment that happens to start with `*`).
- `--devices <file>` as the (uncommon) alternative to the single-file convention, and why the
  single-file form is the default recommendation.
- `#`/`;` as device-file-only comments, distinct from SPICE `*` comments.
- One shared MOSFET constraint worth calling out up front: every MOSFET in one run must share
  the same `r_on` (a `dae-runtime` limitation, not a netlist typo if two different values seem
  to silently collapse to one).
- This chapter is the *index* into the detail chapters (`pwl-devices.md`, `gate-bindings.md`,
  `signals.md`, `block-library.md`) — keep it short, link out rather than duplicating field
  lists here.

## Source material to adapt from
- `crates/general-simulator-cli/src/main.rs`'s module doc comment, roughly lines 1-40 (the grammar
  preamble before the per-kind detail) — this chapter is close to a direct port of that section.
