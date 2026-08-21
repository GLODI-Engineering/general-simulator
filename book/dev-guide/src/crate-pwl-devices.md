# `pwl-devices`: diode and MOSFET models

*(Skeleton — outline below; not yet written.)*

## What goes here
- The 3-segment diode model precisely: breakdown / leakage / forward, the canonical
  reference-slope decomposition (`DiodeCanonical`) that makes the LCP fold possible — link to
  `lcp-formulation.md` rather than re-deriving.
- `Diode::reversed()`: the algebraic identity it satisfies
  ($\mathrm{reversed}().current(v) = -\mathrm{self.current}(-v)$), how it was derived (substitute
  $u=-v$ into each segment), and why it exists (the MOSFET body-diode node-order fix — full
  story with citations, not just the result).
- The `Mosfet` struct: what it models physically, what it deliberately does *not* model
  (`Vgs`/`Vth` as a real signal — link to `switch-model-mosfets.md`).

## Source material to adapt from
- `crates/pwl-devices/src/diode.rs` and `mosfet.rs` — both already carry this depth of
  rationale in their own doc comments; this chapter is largely curation.
- `elspice-pwl`'s journal, the body-diode node-order fix entry, for the full incident account.
