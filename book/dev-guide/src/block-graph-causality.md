# Computational causality: `topological_order`

## Execution order is derived, not declared

A block-diagram netlist can name its inputs in whatever order the author finds clearest — a
`Sum` computing an error signal might read most naturally declared *before* the two sources that
feed it. `dae-runtime` doesn't ask the author to get this right. Once per run, before any step is
solved, `topological_order` computes the actual evaluation order from the `Signal::Block`
dependency graph among the block slice itself:

```rust
fn topological_order(blocks: &[BlockInstance]) -> Result<Vec<usize>, DaeError>
```

— `crates/dae-runtime/src/block_graph.rs:413`. Its own doc comment states the rule directly:

> Derives the causal evaluation order for `blocks` from their own `Signal::Block` dependencies —
> a topological sort of the same-step dependency graph — instead of relying on declaration order:
> a block may name another declared anywhere in the same slice, before or after it.
>
> — `crates/dae-runtime/src/block_graph.rs:396-398`

This is a genuine behavior change from an earlier version of the code, not a restatement of
something that was always true. Before `topological_order` existed, `evaluate_blocks` was a
single forward pass building up an `outputs` map in declaration order — `Signal::Block(name)`
could only resolve if `name` was already in that map, meaning a block genuinely had to be
declared before anything reading it, on pain of an opaque runtime error. The journal entry
recording this fix (`docs/journal/2026-08.md:1347-1355`, "Q1: Is execution order derived from the
block diagram?") is blunt about the prior state: "blocks were evaluated 'in declaration order,'
and `Signal::Block` was documented as requiring its target to be 'declared earlier in the same
slice' — a manual discipline on the netlist author, not something the simulator computed." That
rule is now merely good netlist-authoring style (a reader tracing a diagram top-to-bottom still
benefits from declarations roughly matching data flow) — it is no longer a correctness
requirement enforced or relied on anywhere in `dae-runtime`.

`crates/dae-runtime/tests/topological_order.rs` is the regression test that pins this down. Its
first case is deliberately adversarial about declaration order:

```rust
// SUM is declared FIRST but reads A and B, both declared AFTER it -- illegal under the old
// "declared earlier" rule, now resolved correctly by topological_order.
let blocks = vec![
    BlockInstance { name: "SUM", kind: Sum(vec![1.0, -1.0]),
                    inputs: vec![Signal::Block("A"), Signal::Block("B")] },
    BlockInstance { name: "A", kind: Const(Scalar(7.0)), inputs: vec![] },
    BlockInstance { name: "B", kind: Const(Scalar(3.0)), inputs: vec![] },
];
```

(`crates/dae-runtime/tests/topological_order.rs:51-70`, paraphrased for length — the real test
uses the full `BlockKind`/`Signal` constructors). `SUM` is index `0`, `A` is index `1`, `B` is
index `2`. `run(&blocks)` asserts `outputs["SUM"] == 4.0` (`7.0 - 3.0`) on the very first
resolved step — this only works if `A` and `B` are actually evaluated *before* `SUM` reads them,
despite being declared after it in the source `Vec`.

## Why: the algorithm, traced

`topological_order` is a classic DFS-based topological sort with three-color node marking
(White = unvisited, Gray = on the current recursion stack, Black = fully finished), chosen
specifically because a discovered cycle can be reported as the exact path that closes it rather
than merely the set of nodes a different algorithm would leave stranded (see
`block-graph-cycles.md` for why that distinction mattered enough to drive the algorithm choice).
The core of `visit` (`crates/dae-runtime/src/block_graph.rs:421-460`):

```rust
color[i] = Color::Gray;
stack.push(i);
for signal in &blocks[i].inputs {
    if let Signal::Block(name) = signal {
        if let Some(&dep) = index_of.get(name.as_str()) {
            visit(dep, blocks, index_of, color, stack, order)?;
        }
    }
}
stack.pop();
color[i] = Color::Black;
order.push(i);
```

Tracing it against the `SUM`/`A`/`B` example above, starting the outer loop at index `0` (`SUM`):

| Step | Action | `color` | `stack` | `order` |
|---|---|---|---|---|
| 1 | `visit(0)` (`SUM`): mark Gray, push | `[Gray, White, White]` | `[0]` | `[]` |
| 2 | `SUM`'s first input is `Signal::Block("A")` → `visit(1)` (`A`): mark Gray, push | `[Gray, Gray, White]` | `[0, 1]` | `[]` |
| 3 | `A` has no inputs (`Const`) — loop body doesn't run; pop, mark Black, push to `order` | `[Gray, Black, White]` | `[0]` | `[1]` |
| 4 | Back in `SUM`'s loop: second input `Signal::Block("B")` → `visit(2)` (`B`): mark Gray, push | `[Gray, Black, Gray]` | `[0, 2]` | `[1]` |
| 5 | `B` has no inputs — pop, mark Black, push to `order` | `[Gray, Black, Black]` | `[0]` | `[1, 2]` |
| 6 | Back in `SUM`: no more inputs — pop, mark Black, push to `order` | `[Black, Black, Black]` | `[]` | `[1, 2, 0]` |

Final `order = [1, 2, 0]` — `A`, then `B`, then `SUM`: exactly the causal order the netlist's own
declaration order got backwards. `evaluate_blocks` then iterates `order` directly
(`crates/dae-runtime/src/block_graph.rs:797`, `for &i in order`), so this is not merely a sanity
check computed and discarded — it is the actual per-step iteration order for the rest of the
run.

## Why `Signal::Measure`/`Signal::BlockPrev` never contribute an edge

`visit`'s dependency loop only matches `Signal::Block(name)` — a `Signal::BlockPrev` reference
is silently skipped, contributing nothing to `stack`/`color` at all. This isn't a special case
carved out of the algorithm; it falls out of what `Signal::BlockPrev` *means*. Its own doc
comment states the distinction precisely:

> A named block's own output from the *previous* step (`0.0` before the first step, matching
> every dynamic block's own "starts at rest" convention). Unlike `Signal::Block`, this is not a
> same-step dependency at all — it reads state fixed before this step even starts — so it never
> contributes an edge to the dependency graph `topological_order` ... builds.
>
> — `general-mna/src/block_graph.rs:57-61`

Concretely: `evaluate_blocks`' `resolve` closure treats the two variants completely differently.
`Signal::Block(name)` looks the name up in `outputs`, the map being built up *during this step*,
and fails with `DaeError::UnknownBlockInput` if it isn't there yet (`crates/dae-runtime/src/
block_graph.rs:786-789`). `Signal::BlockPrev(name)` instead reads `prev_outputs` — the complete,
already-finished map returned by the *previous* call to `evaluate_blocks` — falling back to
`SignalValue::Scalar(0.0)` if the name isn't there at all, which is exactly the case on the very
first step before any step has been accepted (`crates/dae-runtime/src/block_graph.rs:790-793`).
Because `prev_outputs` is a value frozen before this step's own evaluation begins, there is no
way for a `BlockPrev` reference to participate in a same-step ordering constraint — it doesn't
matter whether the named block is evaluated before or after the block reading its previous
value, because "previous value" was already fixed before either of them ran this step. This is
why `Signal::BlockPrev` — spelled `prev:` at the netlist-authoring level — is the one legitimate
way to break what would otherwise be a same-step cycle into a legitimate one-sample-delayed
feedback path: routing an edge through it removes that edge from the dependency graph
`topological_order` builds entirely, rather than reordering around it. See
`block-graph-cycles.md` for a worked example where this is the actual fix for a real algebraic
loop.

## What replaced what

The prior rule — "declare sources before sinks, `Signal::Block` must name an earlier block" —
is fully retired as a correctness mechanism. `DaeError::UnknownBlockInput` now means exactly one
thing: "this name doesn't resolve to any block in the graph at all" (a typo, or a genuinely
undeclared reference), never "this block exists but hasn't been evaluated yet" — that second,
previously-conflated failure mode is structurally impossible once `topological_order` has
succeeded, since every `Signal::Block` edge it saw is guaranteed already resolved by the time
`evaluate_blocks` reaches the block that reads it. `DaeError::UnknownBlockInput`'s own doc
comment records this directly: "declaration *position* is no longer a possible cause ...
`topological_order` derives each step's evaluation order from the `Signal::Block` dependency
graph itself, so a block may reference another declared anywhere in the same slice, before or
after it" (`crates/dae-runtime/src/lib.rs:106-110`).
