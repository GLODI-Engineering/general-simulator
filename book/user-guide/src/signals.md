# Signals: same-step references and `prev:`

**A note on this chapter's own title, before anything else**: the skeleton this chapter was
planned from was titled around `meas:`, `prev:`, and same-step references, as if `meas:` were a
third, distinct signal kind alongside the other two. It isn't, and grepping the current parser
confirms it: `general_mna::block_graph::Signal` has exactly two variants,
`Block(String)` (a same-step reference to another declared block) and `BlockPrev(String)` (that
block's own output from the *previous* step — the `prev:` syntax). There is no `Meas`/`meas:`
variant anywhere in the grammar. Reading a circuit quantity into the signal domain is instead
done the same way as everything else in this grammar: declare a
[`kind=phys2sig`](component-reference.md#phys2sig-physical-to-signal-converter) block once
(`PROBE kind=phys2sig node=out`), then reference `PROBE` by name exactly like any other block's
output — a `phys2sig` block is a zero-input source block, not a special inline shortcut. See
[Grammar overview](netlist-grammar.md) and [Gate bindings](gate-bindings.md) for where
`phys2sig`'s write-direction counterpart, `sig2phys`, comes up.

So a block's `in=`/`inputs=` field names one of exactly two things:

- **Another declared block** — read as that block's output computed *this same step*.
- **`prev:<block>`** — that block's own output from the *previous* step (`0.0` before the first
  step, matching every dynamic block's own "starts at rest" convention).

## Ordering is automatic — declare in whatever order reads best

**Blocks are not evaluated in file order.** Each step's actual evaluation order is derived
automatically from the `Signal::Block` dependency graph itself
(`dae_runtime::block_graph::topological_order`, a DFS-based topological sort), so a block may
name another block declared anywhere in the same file, before or after it. If you've seen an
earlier example insisting sources be declared before sinks, that rule no longer applies — it's
still good practice for a human reading the file (left-to-right, like a signal-flow diagram), but
it is not a correctness requirement any more.

## When to reach for `prev:`

`prev:` exists specifically to close a loop *around a block itself*, as opposed to around the
circuit. Two concrete cases that come up constantly:

- A current controller regulating a [`kind=pmsm`](pmsm.md)'s own `id`/`iq` outputs — the
  controller's error signal needs the motor's current, but the motor's next input is the
  controller's own output, so a same-step reference in either direction closes a genuine cycle.
- A PLL's angle estimate feeding the very [`kind=park`](coordinate-transforms.md) block that
  produced the error driving that estimate.

In both cases, routing exactly one edge in the cycle through `prev:<block>` instead of
`Signal::Block` turns a same-step algebraic loop into a legitimate, one-sample-delayed
sampled-data feedback path — the same delay every real digital controller reading its own last
output already has. A minimal, directly runnable illustration (not a full control loop, just the
mechanism): `PROBE` reads a resistor-divider node, and `DELAY` reads `PROBE`'s value from one
step back rather than the current one:

```text
V1 a 0 5
R1 a b 1k
R2 b 0 1k
PROBE kind=phys2sig node=b
DELAY kind=sum inputs=prev:PROBE signs=1
```

Run at `--dt 0.001` for three steps, `DELAY` visibly lags `PROBE` by exactly one row (verified
against the real CLI):

```text
t,V(a),V(b),I(V1),PROBE,DELAY
0.001,5,2.5,-0.0025,0,0
0.002,5,2.5,-0.0025,2.5,0
0.003,5,2.5,-0.0025,2.5,2.5
```

`DELAY` reads `0` at `t=0.001` (before any step has been accepted, `prev:` is `0.0` by
convention) and `2.5` only from `t=0.003` on — one full step behind `PROBE`'s own value.

## What happens if you write a same-step cycle anyway

A genuine same-step cycle among `Signal::Block` references — however indirect, including a block
naming itself — is rejected before any step runs, with the exact closing path spelled out
(verified against the real CLI, a two-block mutual reference):

```text
error: AlgebraicLoop { cycle: ["A", "B", "A"] }
```

`cycle` is the dependency path in order, with the first name repeated at the end to show the loop
closing — a self-referencing block reports `["A", "A"]`. The fix is always the same: find the one
edge in the printed cycle that's conceptually "feedback" rather than "this step's forward signal
flow," and change that particular reference from a plain block name to `prev:<block>`.

## Cross-references

- [Gate bindings](gate-bindings.md) — how a resolved block output ultimately drives an ideal
  switch's gate, once you know how to reference blocks at all.
- [Coordinate transforms](coordinate-transforms.md) and [The PMSM block](pmsm.md) — both worked
  examples use `prev:` at exactly the point this chapter describes, closing a loop around the
  transform/motor block itself.
