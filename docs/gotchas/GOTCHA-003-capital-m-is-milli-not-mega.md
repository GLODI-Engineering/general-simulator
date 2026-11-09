---
id: GOTCHA-003
discovered: 2026-11-09
discovered_by: contributor
scope:
  - crates/general-simulator-cli/**
  - book/user-guide/src/netlist-grammar.md
severity: high
status: documented
reproducibility: full
tags: [netlist, grammar, units, silent-divergence]
---

# GOTCHA-003 — `1M` is one milliohm, not one megohm

## Symptom

A netlist that reads like a megohm resistor simulates as a milliohm one, and
nothing says so. Nothing is misspelled, nothing is rejected, and the run
succeeds — the circuit is simply a billion times away from the one that was
meant, which usually reads as "the solver is wrong" rather than "the deck is
wrong".

```
R1 in out 1M        * one MILLIohm (1e-3), not 1e6
R1 in out 1MEG      * one megohm
```

Both parse. Neither warns.

## Reproduction

A divider makes the value observable — a DC operating point on an RC does not,
because `V(out)` settles to the source regardless of `R`:

```
* div.cir
V1 in 0 1
R1 in out <value>
R2 out 0 1000
```

`V(out) = 1000 / (R1 + 1000)`, so:

```bash
general-simulator div.cir --mode dc
```

| `R1` written as | `V(out)` | implied `R1` |
|---|---|---|
| `1000` | `0.5` | 1000 |
| `1k`, `1K` | `0.5` | 1000 |
| `1m`, **`1M`** | `0.9999990000010001` | **1e-3** |
| `1u`, `1U` | `0.9999999989999999` | 1e-6 |
| `1meg`, `1MEG` | `0.000999000999000999` | 1e6 |

## Root cause

Not a bug: it is SPICE's grammar, which this engine follows deliberately. Suffixes
are **case-insensitive**, so `M` and `m` are the same suffix, and in SPICE that
suffix is `milli`. Mega is the three-letter `MEG`. Every SPICE-family tool behaves
this way; the trap is that `M` is the obvious abbreviation for mega in every other
context an engineer works in, and case-insensitivity removes the one distinction
that would have made `M` and `m` separable.

## Fix or workaround

Write `MEG` for mega, and prefer lowercase `m` for milli so the intent is plain to
a human reader even though the parser does not care. When a value matters, check it
in the output rather than trusting the deck: a divider, or the operating point of
the node in question.

## Prevention

- The grammar chapter now has a **Number suffixes** section with this table, so the
  rule is stated where someone looks up how to write a value rather than only being
  discoverable by being bitten.
- `general-simulator-gui`'s parameter editor mirrors this parser and has a parity
  test over a shared table of cases, so the editor and the engine cannot drift on
  it; it also echoes the parsed SI value back under the field, which turns the
  silent divergence into something visible at entry time.

## References

- `book/user-guide/src/netlist-grammar.md` — "Number suffixes".
- Measured against `general-simulator` built from `74396b5` on 2026-11-09.

## History

- 2026-11-09: Discovered while checking whether the grammar chapter documented
  suffixes at all — it did not, in any form.
