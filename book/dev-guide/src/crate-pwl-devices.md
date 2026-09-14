# `pwl-devices`: ideal-diode and ideal-switch models

Two device *curves*, and nothing else. `crates/pwl-devices/src/lib.rs` is explicit about the
boundary: this crate defines device curves as pure functions of a device's own terminal
voltage/current; folding a set of devices into a whole circuit's `(M, q)` LCP is
`dae-runtime`'s job. The first end-to-end proof of that folding, done manually, lives in
`crates/pwl-devices/tests/two_ideal_diode_circuit.rs`.

## `IdealDiode`: the 3-segment curve

`src/ideal_diode.rs`. Reverse-breakdown conduction below `v_breakdown`, near-zero leakage
(`g_off`) between `v_breakdown` and `v_th`, forward conduction (`g_on`) above `v_th` — five
parameters, continuous at both breakpoints by construction (each segment's offset anchors it
to the leakage segment at the shared boundary). `IdealDiode::new` asserts
`v_breakdown < v_th` (a reversed ordering would silently swap the curve's meaning).

Two independent evaluations of the same curve, and the crate keeps both deliberately:

- `IdealDiode::current(v)` — the direct piecewise formula, branch by branch.
- `IdealDiode::canonical()` — the Chua-Lin canonical decomposition anchored on the leakage
  segment:

$$I(v) = g_{off} \cdot v + \Delta_{on} \cdot z_2 - \Delta_{br} \cdot z_1, \qquad
z_1 = \max(0,\, v_{breakdown} - v), \qquad z_2 = \max(0,\, v - v_{th})$$

  with `delta_on = g_on - g_off`, `delta_br = g_breakdown - g_off`. The `max(0, ...)` terms
  are exactly the LCP `z` variables this diode contributes, each complementary to a guard
  slack (`w1 = v - v_breakdown + z1`, `w2 = v_th - v + z2`); `docs/architecture.md` derives
  why that is equivalent to "exactly one segment active," and `lcp-formulation.md` carries the
  fold into `(M, q)` — not re-derived here. The two formulas agreeing at many sample points
  (the `canonical_matches_direct_piecewise_evaluation` test, including both breakpoints
  exactly) is a real cross-check: they were written independently.

## `IdealDiode::reversed()`: the terminal-order fix

The algebraic identity, stated in the code:

$$\mathrm{reversed}()\mathrm{.current}(v) = -\mathrm{self.current}(-v) \quad \text{for every } v.$$

Derived by substituting `u = -v` into each of the three segments and negating — the result is
a new 3-segment curve with the same shape but mirrored breakpoints and swapped slopes, which
is exactly what `IdealDiode::new` already builds, just called with
`(g_on, -v_th, g_off, -v_breakdown, g_breakdown)`. The invariant `v_breakdown < v_th`
guarantees the reversed curve's own `-v_th < -v_breakdown`, so it is automatically valid.

Why it exists: every PWL device in this crate defines its curve in terms of
`V = V(n1) - V(n2)`, but the *caller's* terminal-order convention can be the mirror image. The
doc comment's rule: **swap the curve, not the netlist nodes.** The concrete incident that
motivated it is recorded in `docs/journal/2026-08.md` (2026-08-20, "`Mosfet` body-diode node
order flipped to plain SPICE `(drain, source)`"): the switch struct used to require netlists
to declare `(source, drain)` so the unmirrored body diode stamped correctly as-is — and 6 of 8
switches in the TIDA-010954 cycloconverter experiment were declared in the natural,
SPICE-conventional `(drain, source)` order, silently putting their body diodes backwards. The
struct's contract changed to remove the footgun, not just document it better.

## `IdealSwitch`: channel + body diode

`src/ideal_switch.rs`. An idealized power switch modeling a real MOSFET's behavior: a
controlled `r_on` channel when gated on, falling back to its intrinsic body diode when gated
off. Named `IdealSwitch` rather than `Mosfet` because it is a PWL companion model, not
BSIM-style device physics — the doc comment is explicit that "MOSFET" is reserved for a
future, not-yet-implemented model, and `docs/journal/2026-08.md`'s 2026-08-29 rename entries
are the full incident account (including the two-step dance where `general-mna`, a read-only
sibling, blocked the full rename until it was updated separately — see
`project-boundaries.md`).

The real physics the model captures, corrected from an earlier version of the doc comment
that described it slightly wrong:

- Channel on/off depends only on `Vgs` vs `Vth`, **not** on the sign of `Vds`. A real
  MOSFET's channel, once on, conducts in both directions almost symmetrically (synchronous
  rectification exploits exactly this). So it is not "`Vds>0` → switch, `Vds<0` → diode" — it
  is "gate on → `r_on` between drain and source for *any* `Vds`", and "gate off →
  body-diode-only behavior for *any* `Vds`" (blocking when `Vds>0`, conducting when
  `Vds < -Vf`). `Vds`'s sign only matters *given* the gate is off.
- Gated off, the body diode (anode at the source, cathode at the drain for an N-channel
  device) still conducts if the circuit pushes current backward through it, independent of the
  now-irrelevant gate command.

Crucially, **which regime applies is not something an LCP resolves.** The gate command is an
external, exogenously-known control input — the caller (`dae-runtime`) knows the gate state
before the circuit is even built and picks one representation up front, per instance, per
timestep (`dae_runtime::solve_dc_with_ideal_switches`). Why the channel state must stay
exogenous (a closed-to-open transition is a *topological* change to the fixed linear system,
not a segment choice) is `switch-model-ideal-switch.md`'s subject; the still-open question of
a genuinely self-triggering switch is tracked in `open-questions.md`.

### The node-order convention and the stamping helper

The netlist declares the device's two terminals **`(drain, source)`** — plain SPICE
convention. The `body_diode` field is declared in its own natural terms (forward conduction
for `V(source) - V(drain) > v_th`), which means the curve as literally stamped between
`(drain, source)`-ordered nodes needs mirroring first. Hence
`IdealSwitch::body_diode_for_drain_source_stamping()`, which returns
`body_diode.reversed()`: every caller stamping the off-state should go through it, never
through `body_diode` directly — the `body_diode` field's own doc comment says "Do **not**
stamp this directly" and points at the helper. The user-facing half of this (the netlist-level
convention, and the failure mode if you get it backwards) is
`book/user-guide/src/pwl-devices.md`, "The node-order convention: `(drain, source)`".

What the model deliberately does **not** model: `Vgs`/`Vth` as a real signal. The gate is not
a voltage compared against a threshold anywhere in this crate — the caller's already-resolved
gate state *is* the input. See `switch-model-ideal-switch.md` for the design argument.

## Source material this was adapted from

- `crates/pwl-devices/src/lib.rs` — the curves-only job boundary.
- `crates/pwl-devices/src/ideal_diode.rs` — the 3-segment curve, the canonical
  decomposition, `reversed()` and its derivation.
- `crates/pwl-devices/src/ideal_switch.rs` — the channel/body-diode model, the naming
  rationale, the node-order convention and the stamping helper.
- `docs/journal/2026-08.md` — the 2026-08-20 body-diode node-order entry and the 2026-08-29
  rename entries.
