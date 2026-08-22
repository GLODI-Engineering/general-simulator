# Design decisions log

*(Skeleton — outline below; not yet written.)*

## What goes here
- A running, dated list of "we considered X, chose Y, because Z" entries — shorter and more
  scannable than the journal (which is chronological narrative), organized by topic instead of
  by date. Candidates already well-documented enough to write up directly:
  - PWL segments via LCP vs. Newton + voltage limiting.
  - MOSFET channel state: exogenous command vs. LCP-resolved (`docs/journal/2026-08.md`,
    "Robustness Q&A...", Q5).
  - Block-graph execution order: derived topological sort vs. manual declaration order
    (same journal entry, Q1/Q2).
  - `Signal::BlockPrev` vs. an alternative that would have let same-step cycles exist with a
    special resolution rule (why the one-sample-delay design was preferred).
  - Per-block RK4 vs. one fused ODE across the whole block graph (once that question's answer
    exists — see `block-graph-rk4.md`).
  - Bench-scale vs. mains-scale for the PFC switching demo, and *not* forcing a "unify MOSFET
    switching" fix under time pressure — decisions made explicitly, not silently, worth
    recording as decisions even when the underlying problem stayed open.
  - **Enforced physical/signal-domain converters, implemented 2026-08-21** (`BlockKind::Probe`/
    `Sig2Gate`/`Sig2Voltage`/`Sig2Current`, `docs/journal/2026-08.md` for the full account) —
    no longer an open question, this entry is now `## Physical/signal-domain converters` proper
    (not just a pointer), since the decision and its rationale are settled: user-requested,
    modeled directly on a reference tool/Simscape's PS-a reference tool/a reference tool-PS Converter split, enforced at
    the netlist/`dae-runtime` level (not just a UI convention) so a violation is a build-time
    error (`DaeError::GateTargetNotSig2Gate`/`SourceNotSig2PhysicalConverter`) regardless of
    which tool produced the netlist.
- Format: one entry per decision, `## <short title>`, `**Considered:**`, `**Chose:**`,
  `**Because:**`, `**Revisit if:**` (conditions that would reopen the question).

## Source material to adapt from
- `docs/journal/2026-08.md`, read end to end — the documentation-planning session's own Q&A
  entry, and everything before it, is largely a sequence of these decisions already reasoned
  through in narrative form; this chapter is the scannable, by-topic distillation of that.
