# Algebraic loop detection

## Why a same-step cycle is a genuine model error

A same-step cycle among `Signal::Block` edges — block `A`'s input traces, however indirectly,
back to `A`'s own output in the *same* evaluation step — has no feedforward evaluation order that
resolves it: whichever of the cycle's blocks is picked to run first needs a value that only
becomes available after some other block in the cycle has already run, which itself needs a
value only available after the first. This isn't a limitation of `topological_order`'s
particular algorithm; it's true of any single-pass evaluation of a same-step dependency graph
with a cycle in it. The fix is consequently always the same: break the cycle by routing one edge
through `Signal::BlockPrev` (`prev:` in netlist syntax), turning it from a same-step dependency
into a one-sample-delayed one — never a solver-side workaround, because there isn't a
mathematically sound one at this layer (an *implicit* algebraic solve across the block graph
would be a different, much larger feature; this project's block graph doesn't attempt it — see
`block-graph-descriptor.md`'s closing section on sampled-data co-simulation).

## The detection mechanism, precisely

`topological_order` uses the same three-color DFS marking described in `block-graph-causality.md`
— White (unvisited), Gray (on the current recursion stack), Black (fully finished) — and a cycle
is detected as a *live back-edge*: an edge from the node currently being visited into a node that
is still Gray, i.e. still on the DFS recursion stack, meaning it is a genuine ancestor of the
current node in this traversal, not just some previously-finished node sharing the same name
coincidentally. From `visit`'s own match on `color[i]`:

```rust
match color[i] {
    Color::Black => return Ok(()),
    Color::Gray => {
        // A back-edge into a node already on the current DFS path: the cycle is that
        // node onward through the rest of the path, with it repeated at the end to show
        // the loop closing.
        let start = stack.iter().position(|&s| s == i).expect(
            "a Gray node is always still on the stack by this function's own invariant",
        );
        let mut cycle: Vec<String> = stack[start..]
            .iter()
            .map(|&idx| blocks[idx].name.clone())
            .collect();
        cycle.push(blocks[i].name.clone());
        return Err(DaeError::AlgebraicLoop { cycle });
    }
    Color::White => {}
}
```

— `crates/dae-runtime/src/block_graph.rs:429-444`. Note precisely what happens here: the cycle is
*reconstructed from the current recursion stack* (`stack[start..]`), not inferred after the fact
from some separately-recorded edge list. Because `stack` at this point holds exactly the chain of
blocks the DFS descended through to reach `i` — in dependency order, each one depended on by the
next — slicing from `i`'s own position onward and then re-appending `blocks[i].name` at the end
produces the *exact closing path*: block names in the order the dependency actually runs,
first name repeated at the end to show visually where the loop closes back on itself, e.g.
`["A", "B", "C", "A"]` read as "A depends on B depends on C depends on A."
`DaeError::AlgebraicLoop`'s own doc comment states this contract directly (`crates/dae-runtime/
src/lib.rs:112-123`), and the regression test that pins the exact shape (not just "an error was
returned") is explicit about not assuming which node the DFS happens to start from:

```rust
let err = run(&blocks).unwrap_err();
match err {
    DaeError::AlgebraicLoop { cycle } => {
        assert_eq!(cycle.first(), cycle.last(), "cycle={cycle:?}");
        assert_eq!(cycle.len(), 4, "cycle={cycle:?}"); // A/B/C plus the repeated closer
        let mut seen: Vec<&str> = cycle[..3].iter().map(String::as_str).collect();
        seen.sort();
        assert_eq!(seen, ["A", "B", "C"], "cycle={cycle:?}");
    }
    other => panic!("expected AlgebraicLoop, got {other:?}"),
}
```

— `crates/dae-runtime/tests/topological_order.rs:99-112`, for a netlist where `A` reads `C`, `B`
reads `A`, `C` reads `B` (a 3-block ring, `crates/dae-runtime/tests/topological_order.rs:81-97`).

## Why DFS over Kahn's algorithm

`topological_order`'s own doc comment states the choice explicitly:

> Implementation: classic DFS-based topological sort with three-color marking ..., chosen over
> Kahn's algorithm specifically because a discovered cycle can be reported as the *exact path*
> that closes it (`["A", "B", "C", "A"]`, read as "A depends on B depends on C depends on A")
> rather than just the set of nodes Kahn's leaves stranded with nonzero in-degree when it gets
> stuck.
>
> — `crates/dae-runtime/src/block_graph.rs:408-412`

Kahn's algorithm (repeatedly removing nodes with zero remaining in-degree) terminates, on a
graph with a cycle, having processed every node *outside* the cycle and stalled with a leftover
set of nodes whose in-degree never reached zero. That leftover set tells you a cycle exists
somewhere among those nodes — but not which edges close it, nor the order in which they close
it, information a netlist author genuinely needs to locate and fix the mistake instead of being
handed a bare set of suspects to manually re-derive the loop from. The DFS approach gets the
closing path "for free" from the recursion stack it already has to maintain to do the sort at
all — no separate post-hoc reconstruction step.

## The self-loop case

A block whose own input references itself — `A`'s `Signal::Block("A")` — is the degenerate
length-1 cycle, and it falls out of exactly the same mechanism with no special-casing: `visit(A)`
marks `A` Gray, pushes it, then immediately encounters `Signal::Block("A")` in `A`'s own inputs
and calls `visit(A)` again, finding `color[A] == Gray` on the very first recursive call. `start`
is `A`'s own (only) position on the stack, so `cycle` is `["A"]` with `blocks[i].name` (`"A"`
again) pushed onto the end, giving `["A", "A"]`. The regression test confirms this exact shape:

```rust
let blocks = vec![BlockInstance {
    name: "A".to_string(),
    kind: BlockKind::Gain(GainValue::Scalar(1.0)),
    inputs: vec![Signal::Block("A".to_string())],
}];
let err = run(&blocks).unwrap_err();
match err {
    DaeError::AlgebraicLoop { cycle } => {
        assert_eq!(cycle, vec!["A".to_string(), "A".to_string()]);
    }
    other => panic!("expected AlgebraicLoop, got {other:?}"),
}
```

— `crates/dae-runtime/tests/topological_order.rs:116-130`.

## What this replaced

Before `topological_order` existed, a same-step cycle was *structurally unrepresentable* in the
old single-pass, declaration-order evaluation scheme — good, in the sense that it couldn't
silently produce a wrong answer — but an attempted one (or, just as often, a genuine typo
resolving to no block at all) surfaced only as `DaeError::UnknownBlockInput`, an error that
conflated two genuinely different causes: "this name doesn't exist anywhere" and "this name
exists but the graph can never make it available before you need it." The 2026-08-21 journal
entry recording the fix is direct about this: "Because evaluation was a single forward pass
building up an `outputs` map, `Signal::Block(name)` could only ever resolve if `name` was already
in that map — meaning a same-step cycle was structurally unrepresentable, but an attempted one
... surfaced only as `DaeError::UnknownBlockInput`, which conflated 'doesn't exist' and 'hasn't
been evaluated yet' into one undifferentiated error" (`docs/journal/2026-08.md:1359-1364`). Today
the two are cleanly separated: `UnknownBlockInput` means only "this name resolves to nothing,"
and `AlgebraicLoop` means only "this name resolves to something, but evaluating it requires
itself" — reported with the exact path, not just the fact of the problem.

## A real example: current control on a PMSM's own `id`/`iq` outputs

`Signal::BlockPrev`'s own doc comment gives, as one of its two worked justifications for why the
variant exists at all, a current controller "regulating a `BlockKind::Pmsm`'s own `id`/`iq`
outputs" (`general-mna/src/block_graph.rs:62-63`). Concretely: `BlockKind::Pmsm` takes three
inputs, in order `vd`, `vq`, `t_load` — rotor-frame stator voltage commands and mechanical load
torque — and produces four outputs including `id`/`iq`, the rotor-frame stator currents
(`general-mna/src/block_graph.rs:2035-2041`). A field-oriented current controller's whole job is
computing `vd`/`vq` *from* an error against `id`/`iq` — e.g. `ERR_D = Sum(inputs=ID_REF, PMSM.id,
signs=1,-1)`, `VD = Pid(in=ERR_D, ...)`, then `PMSM`'s own `vd` input is `Signal::Block("VD")`.
Written this way, with `PMSM.id` read via `Signal::Block`, the dependency graph is exactly a
same-step cycle: `PMSM` depends on `VD` depends on `ERR_D` depends on `PMSM.id`'s own output —
`topological_order` would report this as an `AlgebraicLoop` naming `PMSM`/`VD`/`ERR_D` (and back
to `PMSM`) before any step ever solved, precisely because no evaluation order can produce `vd`
and `id` in the right causal relationship to each other within one step. The fix is exactly what
`Signal::BlockPrev`'s doc comment prescribes: read `PMSM`'s current feedback via `prev:PMSM`
(`Signal::BlockPrev("PMSM")`) instead of `Signal::Block("PMSM")` in `ERR_D`'s inputs. This turns
the loop into a legitimate one-sample-delayed feedback path — the same delay a real digital
current controller already has (it can only ever act on the *previous* sample's measured
current, never the value its own output is about to produce this instant) — and removes the edge
from `topological_order`'s dependency graph entirely, since `BlockPrev` never contributes one.

## A second real example: a PLL's angle feeding its own `Park` block

The same doc comment gives a second, structurally different case: "a PLL's angle estimate
feeding the very `BlockKind::CoordinateTransform` `Park` block that produced its own error
signal" (`general-mna/src/block_graph.rs:63-65`). `BlockKind::CoordinateTransform`'s `park`
variant takes four inputs — three phase quantities plus an angle — and is exactly how a phase-
locked loop tracking a three-phase grid or motor voltage would extract a `d`/`q` representation
to regulate (`general-mna/src/block_graph.rs:1996-1998`). A textbook software PLL's own structure
is inherently this shape: the `Park` transform's `q`-axis output (nominally driven to zero) *is*
the PLL's own phase error, which after a loop filter *becomes* the angle fed back into the same
`Park` block's own angle input on the next evaluation. Wired with `Signal::Block` throughout,
this is again a same-step cycle — `Park`'s angle input depends (through the loop filter and the
error path) on `Park`'s own output. The fix has the identical shape as the PMSM case: the angle
input specifically must be `prev:` (`Signal::BlockPrev`) rather than `Signal::Block`, since a
PLL's angle estimate is, by construction, always one sample behind the error it's currently
correcting — that's what makes it a *loop filter* tracking the phase rather than an
instantaneous, ill-defined self-reference.

Both examples above are grounded directly in `Signal::BlockPrev`'s own doc comment
(`general-mna/src/block_graph.rs:56-65`), which cites them as its own motivating cases; the exact
block/error-signal wiring shown here (which specific `Sum`/`Pid` blocks, which exact netlist
field names) is illustrative of that doc comment's claim rather than transcribed from a single
existing example netlist — no PMSM- or PLL-closed-loop example netlist exercising this specific
cycle was located in `crates/dae-runtime/tests/` or `doc-verify/` during this session, so treat
the general shape (which signal must move to `prev:` and why) as verified, and the precise field
names above as one reasonable way to wire it, not a quoted example from a test.
