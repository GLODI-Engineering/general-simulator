# `pwl-devices`: ideal-diode and ideal-switch models

*(Skeleton — outline below; not yet written.)*

## What goes here
- The 3-segment diode model precisely: breakdown / leakage / forward, the canonical
  reference-slope decomposition (`IdealDiodeCanonical`) that makes the LCP fold possible — link
  to `lcp-formulation.md` rather than re-deriving.
- `IdealDiode::reversed()`: the algebraic identity it satisfies
  ($\mathrm{reversed}().current(v) = -\mathrm{self.current}(-v)$), how it was derived (substitute
  $u=-v$ into each segment), and why it exists (the ideal-switch body-diode node-order fix —
  full story with citations, not just the result).
- The `IdealSwitch` struct: what it models physically (a real MOSFET's on/off channel plus body
  diode), what it deliberately does *not* model (`Vgs`/`Vth` as a real signal — link to
  `switch-model-ideal-switch.md`), and why it's named `IdealSwitch` rather than `Mosfet` ("MOSFET"
  is reserved for a future, not-yet-implemented BSIM-style model — see `docs/journal/`).

## Source material to adapt from
- `crates/pwl-devices/src/ideal_diode.rs` and `ideal_switch.rs` — both already carry this depth
  of rationale in their own doc comments; this chapter is largely curation.
- `general-simulator`'s journal, the body-diode node-order fix entry and the MOSFET-to-ideal-switch
  rename entry, for the full incident accounts.
