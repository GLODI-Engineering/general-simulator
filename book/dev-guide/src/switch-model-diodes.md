# Diodes and MOSFET body diodes: the endogenous case

*(Skeleton — outline below; not yet written.)*

## What goes here
- Restate, briefly (link back rather than re-deriving): diode segment choice is resolved by the
  LCP because every segment shares one reference conductance — an affine-in-`z` change at
  *fixed* topology.
- A gated-off MOSFET's body diode is folded into the exact same diode set, via
  `Mosfet::body_diode_for_drain_source_stamping` (not `body_diode` directly) — explain the
  drain/source node-order mirroring this requires and why (the netlist declares
  `(drain, source)`, the diode's natural forward direction is anode-at-source, so the curve
  needs mirroring to evaluate correctly against the nodes as literally declared).
- What "which segment is active" actually determines for a body diode specifically: natural/
  uncontrolled commutation (synchronous rectification, freewheeling) — contrast this with
  channel on/off, which is never resolved this way (next chapter).

## Source material to adapt from
- `crates/pwl-devices/src/mosfet.rs` module doc comment in full — the physics explanation and
  the node-order rationale (including the "earlier version of this struct had it backwards, a
  real confirmed footgun" account) are already written at exactly this depth.
- `crates/dae-runtime/src/lib.rs`'s `build_with_mosfets` function.
