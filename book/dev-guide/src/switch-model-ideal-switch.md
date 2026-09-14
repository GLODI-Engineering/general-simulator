# Ideal-switch channel state: the exogenous case, and why

*This chapter is about `pwl_devices::IdealSwitch` — a PWL companion model (`r_on` when gated on,
a PWL body diode when gated off), not real BSIM-style MOSFET physics. "MOSFET" is reserved for a
future, not-yet-implemented model of that kind; `IdealSwitch` was renamed from `Mosfet` for
exactly this reason (see `docs/journal/`).*

## The gate is never resolved by the LCP

`IdealSwitch`'s own doc comment (`crates/pwl-devices/src/ideal_switch.rs:29-34`) states the
design directly:

> Which of the two applies at a given timestep is **not** something an LCP resolves — the gate
> command is an external, exogenously-known control input (from a PWM/controller), not a
> function of circuit state, and not a modeled `Vgs` compared against a threshold anywhere in
> this crate: the caller (`dae-runtime`) already knows the gate state before the circuit is even
> built, and picks one of the two representations up front, per instance, per timestep.

There is no `Vgs` vs. `Vth` comparison anywhere in `pwl-devices` or `dae-runtime` — gate state
arrives as an already-decided `GateState` (`On`/`Off`, a re-export of `general_mna::SwitchState`)
for every switch, every step, before the circuit for that step is even assembled.

## The two structural stamps a known gate state picks between

`solve_dc_with_ideal_switches`/`build_with_ideal_switches` (`crates/dae-runtime/src/
lib.rs:699-843`) route each instance to one of two entirely different stamps depending on its
already-known `GateState`:

- **On** → a plain linear switch: `options.set_switch(name, GateState::On)`
  (`lib.rs:832`), reusing `general-mna`'s existing switch mechanism directly — `r_on` between
  drain and source, conducting both directions, with **no complementarity variable at all**. This
  isn't resolved by anything; it's just a resistor for the duration of the step.
- **Off** → the channel opens entirely, and the body diode folds into the diode LCP exactly as
  described in [switch-model-diodes.md](switch-model-diodes.md)
  (`all_diodes.insert(name.clone(), switch.body_diode_for_drain_source_stamping())`, `lib.rs:834`).

Both branches are decided purely from the caller-supplied `GateState`; the LCP solve, when it
runs, only ever has to reason about which *diodes* (ordinary ones, plus every currently-gated-off
switch's body diode) are in which segment — never about switch channel state itself.

## The math reason this can't join the diode LCP

[lcp-formulation.md](lcp-formulation.md) derives why the diode fold works: every segment of a
diode's canonical decomposition shares one fixed reference conductance, so a diode's own segment
choice only ever shifts an affine current-source term at *fixed* topology — the baseline linear
system $A_0$ stays genuinely fixed regardless of which segment `z` resolves to, which is what
makes factoring $A_0$ once and reusing it for every diode's sensitivity vector possible at all.

A switch's channel going from open to `r_on` is not that kind of change. It's the literal
appearance (or disappearance) of a conductance path between two nodes — a genuinely *topological*
change to $A_0$ itself, not a current-source shift at fixed topology. There is no way to
represent "this conductance either exists or doesn't" as an affine function of a complementarity
variable the way a diode's `max(0, ...)` current term can be, because the thing that's changing
isn't a current at fixed topology — it's the topology.

This is precisely why `build_with_ideal_switches` calls `MnaBuilder::with_options` and rebuilds
the *symbolic* system from scratch on every step (`lib.rs:824-841`), rather than reusing one
factored $A_0$ the way the diode fold does — there is no fixed $A_0$ to reuse across a gate
transition, by construction.

## The physical reason it's exogenous anyway, independent of the math

Even setting the math aside, treating gate state as exogenous is the physically correct model,
not a shortcut taken because the LCP mechanism happens not to extend here. This reasoning was
worked out directly in a design-review session and recorded in the project journal
(`docs/journal/2026-08.md`, "Robustness Q&A" entry, 2026-08-21 11:33, Q5 — "Is it not better to
bring MOSFET switching into the same LCP/DAE scheme too?", lines 1420-1445), in response to a
real challenge posed during that review rather than assumed going in:

> In every circuit this crate targets (PWM power converters), the gate signal genuinely *is* an
> externally-commanded control input in the real system too. A real gate driver doesn't discover
> it should switch by solving a complementarity condition on `Vgs` vs. `Vth` — a controller
> commands it directly. Treating it as exogenous is model fidelity to how these systems actually
> work, not a shortcut around a harder problem.

A real gate driver in a real converter is handed a command by a PWM comparator or a digital
controller; it does not sense its own device physics and decide to switch on that basis. Modeling
the gate as an exogenous input mirrors that division of labor exactly: `GateBinding` — covering
`Fixed`, `PwmFixed`, `Vco`, `Pwm`, `Block`, and `VcoPhase` (`docs/architecture.md`, "Timestep
loop") — resolves every gate's state from a controller/carrier signal the same way a real gate
driver would receive one, every step, before the circuit solve happens; the circuit solve then
just uses whatever state it was told.

## What would be a legitimate, different extension

The same journal entry (Q5, "Where the question does point at something real") is explicit that
the challenge wasn't wrong to raise, just aimed at a different feature than what's implemented:

> a genuinely self-triggering ideal switch (a relay, a fuse, a MOSFET operated with no active
> gate signal at all, pure natural commutation) — state genuinely should be resolved from circuit
> state there, but it would still need its *own* formulation (same topology-change problem), not
> simple insertion into the diode `M`/`q`.

A relay, a fuse, or a switch with no active gate driver at all (operating purely on natural
commutation) is a genuinely different device from `IdealSwitch` as modeled here — one whose
on/off state really is a function of circuit conditions, not an external command. Building that
would still face the same topological-change problem this chapter describes (channel
conductance appearing/disappearing isn't an affine current shift at fixed topology, regardless of
what decides when it happens), so it could not simply be inserted into the existing diode LCP
either — it would need its own formulation, not a reuse of this one. This is recorded as a real,
currently-unimplemented extension point rather than dismissed; see
[open-questions.md](open-questions.md).

## What was not possible to verify further

The journal entry above is the primary source for the design rationale, and it is itself
first-hand (a live design-review Q&A, logged the same session, grounded in direct reading of
`ideal_switch.rs`'s own doc comment rather than from memory — the entry says as much). No
additional independent source beyond this journal entry and the `IdealSwitch` doc comment it
quotes was found or needed to state the "why exogenous" rationale confidently; nothing in this
chapter asserts a claim beyond what those two sources state directly.
