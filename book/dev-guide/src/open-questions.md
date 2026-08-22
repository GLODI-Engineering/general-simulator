# Open extension points

*(Skeleton — outline below; not yet written.)*

## What goes here
- A genuinely self-triggering ideal switch (relay, fuse, pure natural-commutation MOSFET with
  no active gate drive) — state-dependent, but *not* foldable into the diode LCP for the same
  topology-change reason MOSFET channel switching isn't (see `switch-model-mosfets.md`); would
  need its own formulation. Left open deliberately, not attempted under time pressure.
- Per-instance MOSFET `Ron` (today every MOSFET in one call shares `shared_r_on`).
- MIMO `StateSpace`/`TransferFunction` in the block graph (the underlying `continuous_blocks::
  StateSpace` type already supports general `(A,B,C,D)`; `block_graph.rs`'s own dispatch
  currently hardcodes single-input/single-output).
- PFC Stage 2's own open bug (q-axis current divergence) as a currently-open, actively-being-
  debugged item — link to the `internal-archive` experiment's README rather than
  duplicating the account here; update this entry once resolved.
- Each entry: what it would take, why it wasn't done now (genuinely out of scope vs.
  deliberately deferred vs. blocked on a decision), and what would make a contributor the right
  person to pick it up.

## Source material to adapt from
- `docs/architecture.md`'s "Status" section's own "Open" list.
- Any experiment README's own Caveats section noting an `elspice-pwl`-side limitation.
- `design-decisions.md`'s "Revisit if" conditions, cross-linked both directions.
