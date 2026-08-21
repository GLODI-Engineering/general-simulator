# `dae-runtime`: circuit assembly and the transient loop

*(Skeleton — outline below; not yet written.)*

## What goes here
- This crate's exact job boundary, stated precisely (and defended): LCP-based mode selection
  for piecewise-linear devices, plus compiling continuous blocks into descriptor-DAE fragments
  — netlist parsing stays in `spice-core`, linear-device MNA stamping stays in `elspice-mna`.
- A map of the module split: `lib.rs` (diode-only transient loop, the core LCP fold),
  `block_graph.rs` (MOSFET + block-graph transient loop, gate resolution, causality/cycles),
  `step_control.rs` (adaptive stepping), `closed_loop.rs` (the lighter fixed-topology
  convenience), `topology.rs`, `linsolve.rs`.
- Forward-reference every other chapter that already covers a piece of this crate in depth
  (LCP fold, DAE integration, ringing, switch model, block-graph causality/cycles/RK4) rather
  than repeating them — this chapter's job is orientation, not re-explanation.

## Source material to adapt from
- `AGENTS.md`'s "Project boundaries" section for the job-boundary framing.
- Each source file's own module doc comment for the module map.
