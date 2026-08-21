# Dynamic blocks: one RK4 per block, wired by causality

*(Skeleton — outline below; not yet written. This chapter answers a question not yet asked in
the doc-planning conversation as of this skeleton's creation — check whether it was answered
in a later turn before writing, and pull that answer directly if so, the same way the previous
two chapters did.)*

## What goes here (best current understanding, to verify against that answer before publishing)
- Each dynamic block (`Pid`/`StateSpace`/`TransferFunction` via `StateSpace::rk4_step`; `Pmsm`
  via its own bespoke `step()`) integrates *independently*, once per circuit step, with its own
  state carried in `BlockState` — there is no single fused ODE/RK4 across the whole graph.
- Why that's correct, not just simpler: `evaluate_blocks` already establishes a causal
  evaluation order (`topological_order`); a block's inputs are fully resolved numbers by the
  time it's stepped, so integrating each block against those fixed-for-this-step inputs is
  exact for the sampled-data model this crate implements — a real digital controller's own
  discrete update *is* "read inputs, step my own state," not part of one continuous joint ODE
  with the plant.
- `Pmsm` as the one case worth walking through in detail: its whole nonlinear vector field
  (including the bilinear speed/current coupling) is evaluated *inside* one `step()` call, so
  RK4's own intermediate stages naturally handle the coupling correctly — contrast this with
  what *would* go wrong if `id`/`iq` dynamics were split across two separate blocks each
  needing the other's current-step output (a same-step algebraic loop, per the causality
  chapters above).
- Where the "sampled-data co-simulation" framing comes from and why it's named that
  specifically: the block graph and the circuit are two separate discrete-time systems, each
  reading the other's *previous* result, not one jointly-solved continuous system.

## Source material to adapt from
- `crates/continuous-blocks/src/state_space.rs`'s `rk4_step`.
- `crates/continuous-blocks/src/pmsm.rs` module doc comment (the bilinear-coupling/RK4-stages
  argument is already written there in detail).
- `crates/dae-runtime/src/block_graph.rs` module doc comment, "Sampled-data co-simulation."
- Whatever answer this question actually received later in the doc-planning conversation —
  check first; don't re-derive if it's already been given.
