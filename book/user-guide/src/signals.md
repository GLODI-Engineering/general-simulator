# Signals: `meas:`, `prev:`, and same-step references

*(Skeleton — outline below; not yet written.)*

## What goes here
- The three signal kinds a block `in=`/`inputs=` field can name: another block (same step),
  `meas:<node>` (the circuit's own previous-step state), `prev:<block>` (any block's own
  previous-step output).
- **Ordering is now automatic**: a block may reference any other block in the same graph,
  regardless of declared position — the evaluator derives a causal order from the dependency
  graph itself. State this plainly and early; it corrects the "declare sources before sinks"
  habit from earlier examples circulating before this changed.
- When to reach for `prev:` specifically: closing a loop *around a block itself* (a controller
  regulating a `Pmsm`'s own `id`/`iq`, or a PLL angle feeding the very `Park` block that
  produced its error) — a same-step reference there is a genuine algebraic loop, not just bad
  style.
- What happens if you *do* write a same-step cycle: the exact error you'll see
  (`AlgebraicLoop`, with the closing path spelled out) and the fix (route one edge through
  `prev:`).

## Source material to adapt from
- `crates/dae-runtime/src/block_graph.rs`: `Signal` enum doc comment, `topological_order`'s own
  doc comment (has the cycle-detection explanation already written in detail).
- This is a good chapter to pull near-verbatim from the architecture Q&A conducted with the
  user during this doc-planning session — the "is order derived from the block diagram" and
  "how are algebraic loops detected" exchange is essentially a finished draft of this page's
  narrative, minus the code citations trimmed to user-appropriate detail.
