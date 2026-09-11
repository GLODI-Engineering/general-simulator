# Grammar overview

*(Skeleton — outline below; not yet written.)*

## What goes here
- The core convention: every device/block declaration is an ordinary SPICE comment line
  (`*`-prefixed) with a `name kind=<kind> field=value ...` payload; a line without `kind=` is
  left alone (including a genuine comment that happens to start with `*`).
- `--devices <file>` as the (uncommon) alternative to the single-file convention, and why the
  single-file form is the default recommendation.
- `#`/`;` as device-file-only comments, distinct from SPICE `*` comments.
- One shared ideal switch constraint worth calling out up front: every ideal switch in one run must share
  the same `r_on` (a `dae-runtime` limitation, not a netlist typo if two different values seem
  to silently collapse to one).
- `ic=` on a `C` or `L` card, with both sign conventions spelled out (a capacitor's is first
  node minus second; an inductor's is the current from first node to second, matching
  `I(<name>)`) and the semantics stated plainly: it is an **assignment**, so the declared states
  start at their declared values and every other unknown starts at rest — the operating point is
  skipped, not solved around the declaration. Contradictions are reported; see
  `general-mna`'s `MnaSystem::initial_state`.
- What a device card's trailing parameters are checked against, per device letter, and the short
  list of tokens that deliberately are not checked (a diode's model name above all) — port the
  table and the "known limits" list from `general-mna`'s README rather than re-deriving them.
- This chapter is the *index* into the detail chapters (`pwl-devices.md`, `gate-bindings.md`,
  `signals.md`, `block-library.md`) — keep it short, link out rather than duplicating field
  lists here.

## Source material to adapt from
- `crates/general-simulator-cli/src/main.rs`'s module doc comment, roughly lines 1-40 (the grammar
  preamble before the per-kind detail) — this chapter is close to a direct port of that section.
