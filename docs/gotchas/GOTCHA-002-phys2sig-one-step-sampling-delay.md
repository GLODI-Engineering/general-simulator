---
id: GOTCHA-002
discovered: 2026-11-06
discovered_by: contributor
scope:
  - crates/dae-runtime/src/block_graph.rs
  - doc-verify/phys2sig/**
severity: medium
status: documented
reproducibility: full
tags: [block-graph, phys2sig, measurement, co-simulation]
---

# GOTCHA-002 — A `phys2sig` column is one step behind the quantity it reads, and a steady-state check cannot see it

## Symptom

A `kind=phys2sig` column disagrees with the `V(...)`/`I(...)` column it reads, in the same CSV
row. Any measurement, controller or comparison that goes through a probe is one sample behind
the circuit, while the netlist looks like it reads the circuit directly.

Diagnosed once, wrongly, as "every CSV row's value is one $\Delta t$ behind its own timestamp"
(`general-simulator#21`). It is not: a circuit unknown and a signal-domain block that does not
read the circuit are both the value at the row's own $t$. Only the probe lags.

## Reproduction

```text
V1 in 0 5
R1 in out 1k
R2 out 0 1k
PROBE1 kind=phys2sig node=out
```

```sh
general-simulator divider.cir --mode transient --tfinal 4e-3 --dt 1e-3
```

```text
t,V(in),V(out),I(V1),PROBE1
0.001,5,2.5,-0.0025,0
0.002,5,2.5,-0.0025,2.5
0.003,5,2.5,-0.0025,2.5
```

`PROBE1` is `V(out)` as of $t - \Delta t$, and `0` in the first row.

## Root cause

The block graph is evaluated *before* the circuit is solved: a block's output may decide a gate
state or a source magnitude for the step about to be solved, so it cannot depend on that step's
own solution. `evaluate_blocks` is therefore handed `point_prev`, and
`block_graph::phys2sig_value` reads the previous accepted step's operating point.

Deliberate, not a defect — the same one-sample delay a real digital controller sampling at the
top of its interrupt has. Recorded as a design choice in
`book/dev-guide/src/block-graph-descriptor.md` and `docs/architecture.md`.

## Fix or workaround

There is nothing to fix in the engine, and no flag that removes the delay. Choose `--dt` small
enough that one step's phase lag is acceptable in whatever loop is closed through the probe. A
`prev:<block>` reference adds a *second* step on top of it, which is worth remembering when a
chain reads a probe through one.

## Prevention

Two traps made this survive as long as it did, and both are now covered:

1. **A settled circuit hides it after row one.** `doc-verify/phys2sig`'s original test asserted
   on the *last* row of an already-settled divider, so it passed while the entry it verifies
   claimed `PROBE1` tracks `V(out)` "exactly". It now runs the same netlist a second time at a
   coarse `dt` and asserts the per-row relation.
2. **A spot check near a sinusoid's peak cannot tell a one-step shift from none.** Near the peak
   $\sin(t)\approx\sin(t-\Delta t)$; only the maximum error over a full cycle separates the two
   hypotheses. That is what led to the wrong diagnosis above, and
   `crates/general-simulator-cli/tests/csv_row_time_alignment.rs` now compares the *worst* row
   of each, in both directions — it pins that there is no global offset *and* that the probe's
   own offset is exactly one step, so a fix for either claim cannot silently break the other.

## References

- `general-simulator#21` — the original (incorrect) report, closed with this measurement.
- `crates/dae-runtime/tests/phys2sig_block.rs` — already asserted the per-step relation before
  any of this; the behaviour was tested and undocumented, not untested.
- `book/user-guide/src/reading-output.md` — "What time a row's values belong to".

## History

- 2026-11-06: Discovered while disproving `general-simulator#21`; documented, not fixed (by
  design).
