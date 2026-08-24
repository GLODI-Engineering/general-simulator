# Blocks as a descriptor-DAE fragment

*(Skeleton — outline below; not yet written.)*

## What goes here
- Every dynamic block compiles to a `StateSpace`: the same descriptor-DAE shape `general-mna`
  uses for circuits ($A x + K \dot x = B u$, `K` called `e` for descriptor systems in code).
- Why this is the organizing idea of the whole project, not just an implementation detail of
  one crate: a state-space block with $K=I$ is ordinary state-space; a "Descriptor State-Space"
  block with $K=E$ is textually identical to `general-mna`'s own convention; a transfer function
  is realized once (controllable canonical form) into $(A,B,C,D)$ then handled identically.
- What's deliberately *not* folded into this shape and why: `Vco` (wraparound is a genuine
  discontinuity), `Hysteresis` (a genuine discrete latch), `Pmsm` (bilinear coupling — its own
  chapter, `block-graph-rk4.md`) — piecewise/discontinuous behavior uses the same
  segment-plus-guard machinery as PWL devices, or a bespoke `step()`, not a second mechanism.

## Source material to adapt from
- `docs/architecture.md`'s "One descriptor system for circuit and continuous blocks alike"
  section — this chapter is close to a direct port.
- `crates/continuous-blocks/src/lib.rs` and `state_space.rs` module doc comments.
