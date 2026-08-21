# Algebraic loop detection

*(Skeleton — outline below; not yet written — but a full draft already exists in the journal,
see below.)*

## What goes here
- Why a same-step cycle among `Signal::Block` edges is a genuine model error (no feedforward
  evaluation order can resolve it), and why the fix is always "route one edge through `prev:`,"
  never a solver-side workaround.
- The detection mechanism precisely: three-color DFS marking; hitting a Gray node mid-recursion
  is a live back-edge; the cycle is reconstructed from the current recursion stack, not
  inferred after the fact.
- Why DFS was chosen over Kahn's algorithm specifically: Kahn's leaves you with "these nodes
  have nonzero in-degree" when it gets stuck, which isn't the same as "here is the cycle" — DFS
  gives the exact closing path (`["A","B","C","A"]`) directly from the stack.
- The self-loop case as the degenerate length-1 cycle (`["A","A"]`), and why it falls out of
  the same mechanism for free rather than needing a special case.
- Contrast with the *old* failure mode this replaced: a same-step cycle used to be
  structurally unrepresentable (good), but an attempted one — or a genuine typo — surfaced only
  as an opaque `UnknownBlockInput` at first evaluation, with no way to tell which cause it was.

## Source material to adapt from
- **`docs/journal/2026-08.md`, entry "Robustness Q&A..." (2026-08-21), Q2** — same journal
  entry as `block-graph-causality.md`, the algebraic-loop half.
- `crates/dae-runtime/src/block_graph.rs`: `topological_order`'s cycle-reconstruction code,
  `DaeError::AlgebraicLoop`'s doc comment.
- `crates/dae-runtime/tests/topological_order.rs`'s three cycle tests (3-block cycle, self-loop,
  cycle broken by `prev:`) as the worked examples for this chapter.
