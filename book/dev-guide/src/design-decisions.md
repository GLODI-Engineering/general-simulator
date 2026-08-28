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
    modeled directly on a real block-diagram tool's own physical/signal converter split,
    enforced at the netlist/`dae-runtime` level (not just a UI convention) so a violation is a build-time
    error (`DaeError::GateTargetNotSig2Gate`/`SourceNotSig2PhysicalConverter`) regardless of
    which tool produced the netlist. **Revised 2026-08-27**: `Sig2Gate` was removed and merged
    into `Sig2Voltage` (a MOSFET's gate is itself a voltage, not a distinct discrete-actuation
    signal domain — no dedicated gate-only converter needed); `DaeError::GateTargetNotSig2Gate`
    was renamed `GateTargetNotSig2Voltage` to match. Two converters remain (`Sig2Voltage`/
    `Sig2Current`), not three.
  - **Signal-domain `pwc`/`pwl` source naming, resolved 2026-08-23** (`docs/journal/2026-08.md`,
    `2026-08-23 15:24`/`15:52`) — no longer an open question, this entry is now
    `## Signal-domain source naming: pwc vs. pwl` proper (not just a pointer): adding
    `SIN`/`PULSE`/`EXP`/`PWL`/`SFFM` to the signal domain (parity with `general-mna`'s own
    electrical-domain source forms) collided with the *existing* `BlockKind::Pwl`, which was
    piecewise-*constant* (a step-schedule block, not the piecewise-*linear* interpolation the
    name implies). User's own proposed resolution: rename the old block `Pwc`, freeing `Pwl` for
    real SPICE-matching piecewise-linear semantics — settled, implemented, every prior use
    mechanically renamed and re-verified byte-identical.
- Format: one entry per decision, `## <short title>`, `**Considered:**`, `**Chose:**`,
  `**Because:**`, `**Revisit if:**` (conditions that would reopen the question).

## Signal-domain source naming: pwc vs. pwl

**Considered:**
1. Keep `kind=pwl` meaning what it already meant (piecewise-constant, used for step reference
   schedules), and give the new SPICE-matching piecewise-linear source a different name (e.g.
   `pwl_lin`, `spice_pwl`).
2. Change `kind=pwl`'s own interpolation to real piecewise-linear, matching the electrical
   domain's `PWL(...)` source exactly, letting every existing user of the name adopt the new
   (correct-per-the-name) behavior automatically.
3. Rename the existing piecewise-constant block to `pwc`, freeing `pwl` for the new
   piecewise-linear source.

**Chose:** Option 3 (user's own proposal).

**Because:** Option 1 keeps two similarly-named blocks (`pwl`/`pwl_lin`) with *different*
interpolation behavior, exactly the kind of "easy to mix up because they sound like the same
thing" trap `general-mna`'s own `PwlPoints` doc comment already warns readers about for the
unrelated block-graph/electrical-domain naming overlap. Option 2 is a silent breaking change:
every existing reference-schedule netlist in this project's own `internal-archive`
sibling repo (e.g. a speed-step schedule `REF kind=pwl points=[[0,800],[0.05,1500]]`) relies on the
*step* behavior — reinterpreting it as a ramp would silently corrupt every one of those
experiments' own recorded results without a single line of `general-simulator` itself reporting an
error. Option 3 is the only one that (a) makes `pwc`/`pwl` self-document the actual
interpolation-style distinction directly in the name (constant vs. linear — the same "c"/"l"
distinction SPICE dialects don't need since they only ever had the linear one), (b) frees `pwl`
to mean exactly what it means everywhere else (real SPICE PWL semantics, matching
`general-mna`'s own source exactly, so a signal-domain reference and a `V`/`I` source built from
the same breakpoints are the same waveform), and (c) is a purely mechanical, behavior-preserving
rename for every existing use — every `internal-archive` netlist using the old
piecewise-constant block was renamed `kind=pwl` → `kind=pwc` and re-verified byte-identical
against its pre-rename baseline, not silently reinterpreted.

**Revisit if:** A future signal-domain source needs a third interpolation style (e.g. cubic/
spline breakpoints) and the one-keyword-per-style convention (`pwc`/`pwl`) stops scaling
cleanly — at that point, consider a single parameterized block (e.g. `kind=breakpoints
interp=constant|linear|cubic`) instead of adding a fourth bare keyword.

## Source material to adapt from
- `docs/journal/2026-08.md`, read end to end — the documentation-planning session's own Q&A
  entry, and everything before it, is largely a sequence of these decisions already reasoned
  through in narrative form; this chapter is the scannable, by-topic distillation of that.
