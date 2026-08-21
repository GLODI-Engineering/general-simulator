# Computational causality: `topological_order`

*(Skeleton — outline below; not yet written — but a full draft already exists in the journal,
see below.)*

## What goes here
- Execution order is derived, not declared: a DFS-based topological sort over the
  `Signal::Block` dependency graph among one `Vec<BlockInstance>`, computed once per run before
  any step is solved.
- Why `Signal::Measure`/`Signal::BlockPrev` never contribute an edge: both read state fixed
  *before* the current step starts (the circuit's previous operating point, or any block's own
  previous output), so neither can ever participate in a same-step ordering constraint — this
  is definitional, not a special case bolted on.
- What this replaced: the old "declare sources before sinks, `Signal::Block` must name an
  earlier block" rule, and why that rule is now merely good style, not a correctness
  requirement.
- Worked trace of the algorithm on a small example (3-4 blocks, one deliberately declared
  out of dependency order) — show the color-marking/stack state at each step, not just the
  final order.

## Source material to adapt from
- **`docs/journal/2026-08.md`, entry "Robustness Q&A: execution order, algebraic loops, the
  main loop, switch resolution" (2026-08-21), Q1** — a full draft answer already written there,
  close to publication quality; this chapter is largely expanding it with the worked trace.
- `crates/dae-runtime/src/block_graph.rs`: `topological_order`'s own doc comment and body.
- `crates/dae-runtime/tests/topological_order.rs` for the worked-example candidates (the
  "block declared before its dependency" test is a ready-made small example).
