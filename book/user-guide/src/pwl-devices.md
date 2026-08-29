# PWL devices: diodes and ideal switches

*(Skeleton — outline below; not yet written.)*

## What goes here
- `kind=diode` field reference (`g_breakdown`, `v_breakdown`, `g_off`, `v_th`, `g_on`) with a
  small labeled sketch of the 3-segment I-V curve (breakdown / leakage / forward) and which
  field controls which piece.
- `kind=mosfet` field reference (the netlist keyword itself is unchanged; the underlying model
  is `IdealSwitch`, an ideal switch — not a real MOSFET, which is a separate, not-yet-implemented
  future model): same diode fields for the body diode, plus `r_on`, plus every `gate=` variant
  (link to `gate-bindings.md` for the full detail rather than repeating).
- **The node-order convention**, called out prominently: plain SPICE `(drain, source)` — this
  bit a real user before (see the dev guide's gotcha writeup) and deserves to not be buried.
- A worked numeric example of picking diode parameters from a real datasheet-style spec
  (forward voltage, breakdown voltage) — even a simple one prevents a lot of confusion.

## Source material to adapt from
- `crates/pwl-devices/src/ideal_diode.rs` and `ideal_switch.rs` doc comments — both already
  carry the real physics explanation and the node-order rationale in detail.
- `docs/gotchas/` in this repo (check for the body-diode node-order entry) and the
  `internal-archive` journal entry about the MOSFET-to-ideal-switch rename, for the
  "why this is worth calling out" framing.
