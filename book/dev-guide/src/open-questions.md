# Open extension points

*(Skeleton — outline below; not yet written.)*

## What goes here
- A genuinely self-triggering ideal switch (relay, fuse, pure natural-commutation MOSFET with
  no active gate drive) — state-dependent, but *not* foldable into the diode LCP for the same
  topology-change reason MOSFET channel switching isn't (see `switch-model-mosfets.md`); would
  need its own formulation. Left open deliberately, not attempted under time pressure.
- Per-instance MOSFET `Ron` (today every MOSFET in one call shares `shared_r_on`).
- **Enforced physical/signal-domain converters at the netlist level** (requested by the user,
  2026-08-21, drawing the explicit a reference tool/a reference tool analogy: Simscape's PS-a reference tool Converter /
  a reference tool-PS Converter blocks — a physical port and a signal port are type-distinct there and
  cannot be wired together directly). Today `elspice-pwl` has no such boundary at all:
  `Signal::Measure` reads `V(node)`/`I(branch)` directly into *any* `in=`/`inputs=` field with
  no intermediate block, and a `GateBinding` reads a signal-domain block's output directly to
  decide a MOSFET's exogenous gate state (Q4/Q5's mechanism) — both are unmarked domain
  crossings. There is also currently no crossing *at all* in the signal-to-physical direction
  for a continuous quantity (only the discrete gate command) — a block cannot drive an
  independent voltage/current source's own magnitude, the gap the
  `elspice-pwl-buck-dc-motor-cascade` experiment's own README already documents ("the DC motor
  does not electrically load this circuit... there is no path for a block to inject current
  back into the MNA system").

  Requested design, to become mandatory (not optional) at the netlist level, with the intent
  that a future UI enforces the same rule visually (a physical port literally cannot be wired
  to a signal port without dropping a converter block between them):
  - **PS-to-Signal** (read): every crossing from a circuit quantity into the signal domain —
    every `V(node)`/`I(branch)` read, *and* the gate-state read a `GateBinding` currently does
    directly against a signal block's output — must go through an explicit, named converter
    block declared in the netlist, referenced by name like any other block afterward. No bare
    `meas:`-style inline reference survives as a general-purpose input source.
  - **Signal-to-PS** (write): a new converter kind that takes a signal-domain block's output and
    drives an independent voltage or current source's own magnitude with it, closing the
    write-direction gap above — genuine bidirectional physical/control coupling, not
    observation-only.
  - Implementation touches: new `BlockKind` variant(s) for the PS-to-Signal read converter (the
    underlying mechanism is the same `point_prev` lookup `Signal::Measure` already does — this
    is a type-discipline change, not a new numerical capability, for that half); a `GateBinding`
    change so every variant names a converter block instead of a raw signal block; a genuinely
    new mechanism for Signal-to-PS driving a source (a source's own value needs to become a
    per-step-overridable symbol the way a `TransientFunction`-driven source or a diode's
    `Ioff` already is — likely an `elspice-mna` cross-repo question for how such a source is
    declared in netlist text, needing the same confirm-before-editing discipline as any other
    sibling-repo change).
  - Deliberately not implemented yet as of this entry — scoped and design-confirmed with the
    user first (in-progress as of 2026-08-21; update this entry once landed, and check
    `docs/journal/2026-08.md` for the implementation account).
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
