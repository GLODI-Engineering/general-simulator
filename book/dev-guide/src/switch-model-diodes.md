# Diodes and ideal-switch body diodes: the endogenous case

## Recap: why diode segment choice is LCP-resolved

[lcp-formulation.md](lcp-formulation.md) covers this in full; the short version needed here is
that every segment of a diode's canonical decomposition shares one fixed reference conductance
`g_off`, so switching segments only ever shifts an affine current-source term at *fixed*
topology — which is exactly what keeps the baseline linear system $A_0$ fixed regardless of `z`,
and exactly what makes folding a whole circuit's worth of diodes into one simultaneous LCP solve
possible. This chapter is about what that mechanism resolves for a diode specifically, and how a
gated-off ideal switch's own body diode reuses it unchanged.

## The body diode joins the same diode set

`pwl_devices::IdealSwitch` (`crates/pwl-devices/src/ideal_switch.rs:49-59`) carries a `body_diode:
IdealDiode` field — an ordinary `IdealDiode`, declared in its own natural terms: anode at the
switch's source, cathode at its drain, forward conduction for `V(source) - V(drain) > v_th`, the
same body-diode orientation every real N-channel MOSFET has. When `dae-runtime` builds the
circuit for a step where a given switch is gated off, it doesn't invent a separate mechanism for
this diode — it inserts it directly into the same `diodes: BTreeMap<String, IdealDiode>` every
ordinary netlist `D` diode lives in (`build_with_ideal_switches`, `crates/dae-runtime/src/
lib.rs:817-843`: `all_diodes.insert(name.clone(),
switch.body_diode_for_drain_source_stamping())`, keyed by the switch's own instance name). From
that point on, `fold_and_solve` treats it exactly like any other diode — same canonical
decomposition, same $w_j$/$\gamma_{kj}$ coupling into every other diode (and every other
gated-off switch's body diode) in the circuit, same simultaneous LCP solve. There is no
switch-specific code path once this insertion happens.

## The node-order mirroring, and why it's needed

The netlist declares an ideal switch's two terminals in plain SPICE-conventional `(drain,
source)` order — not `(source, drain)`, which is the order `IdealDiode`'s own `body_diode` field
is defined in. Stamping `body_diode` directly against nodes declared `(drain, source)` would
evaluate the diode's curve against `V = V(drain) - V(source)`, but the diode's own forward
direction is defined for `V(source) - V(drain) > v_th` — exactly backwards.

`IdealSwitch::body_diode_for_drain_source_stamping` (`ideal_switch.rs:66-74`) fixes this by
mirroring the *curve*, not the netlist's own node order: it returns `self.body_diode.reversed()`
— `IdealDiode::reversed()` (`crates/pwl-devices/src/ideal_diode.rs:78-104`) substitutes `u = -v`
into all three segments and negates, which for a diode is exactly equivalent to swapping which
physical terminal is "anode" without touching the underlying physics at all: `reversed().current(v)
== -self.current(-v)` for every `v`, checked directly against that exact identity, sampled across
every segment including both breakpoints (`ideal_diode.rs:171-188`,
`reversed_satisfies_its_own_defining_identity`). Every caller stamping a switch's off-state uses
this method, never `body_diode` directly (`ideal_switch.rs:54-57`'s own doc comment says so
explicitly) — `build_with_ideal_switches` follows that rule (`lib.rs:834`).

**This wasn't always the contract, and getting it wrong was a real, confirmed bug, not a
hypothetical one.** `IdealSwitch`'s own module doc comment records it directly
(`ideal_switch.rs:44-48`): an earlier version of this struct required the netlist to declare
switches `(source, drain)` instead of the SPICE-conventional `(drain, source)`, so the
*unmirrored* `body_diode` stamped correctly as-is. That earlier convention was a real footgun —
6 of 8 switches in the TIDA-010954 cycloconverter experiment were declared in the natural,
SPICE-conventional `(drain, source)` order (the order anyone writing a netlist would reach for
without being told otherwise) and so had their body diodes backwards, silently. The struct's
contract was changed specifically to remove that footgun at the source — requiring the *caller*
to mirror the curve via a clearly-named method, rather than requiring every netlist author to
remember an unusual node-order convention forever. `crates/pwl-devices/src/
ideal_diode.rs:190-206`'s own test
(`reversed_gives_correct_low_side_switch_behavior_at_spice_conventional_node_order`) checks the
corrected behavior directly against the same TIDA-010954 low-side-switch parameters that exposed
the original bug: `v = V(drain) - V(source) = +10` blocks (near-zero leakage), `v = -10` conducts
strongly — the correct low-side body-diode behavior with the netlist declared in the order anyone
would naturally use.

## What "which segment is active" means for a body diode specifically

For an ordinary discrete diode, segment choice answers "is this device in breakdown, leaking, or
forward-conducting." For a gated-off switch's body diode, the same three segments answer a more
specific physical question: **natural, uncontrolled commutation** — synchronous rectification's
freewheeling phase, where the channel is off but current is still flowing somewhere in the
circuit that has to go through the body diode instead. Nothing about *when* this happens is
decided by the gate signal or by any explicit logic in `dae-runtime` — it falls out of the LCP
solve the same way any other diode's segment does, driven purely by whatever the rest of the
circuit's operating point demands at that instant. A half-bridge leg with both switches gated off
during a dead-time window, for instance, has its output current routed entirely through whichever
switch's body diode the LCP resolves into forward conduction — the physically correct behavior
for that condition, discovered by the solve rather than programmed in as a special case.

This is a genuinely different kind of "on/off" from the switch's own *channel* state, which is
never resolved this way — see [switch-model-ideal-switch.md](switch-model-ideal-switch.md) for
why the channel needs an entirely different mechanism.
