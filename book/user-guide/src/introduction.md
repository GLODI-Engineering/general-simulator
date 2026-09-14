# Introduction

`general-simulator` is an educational Rust simulator for piecewise-linear (PWL) circuits and
mixed circuit/block-diagram systems — the kind of deck a power-electronics converter needs: an
ideal diode or switch, a PWM modulator, a PID or state-space controller closing the loop around
it, all in the same run.

**The one-paragraph pitch**: it never runs Newton-Raphson on device physics, so it never needs
SPICE-style voltage limiting either. Every active device is modeled as piecewise-linear — a
diode's three conduction segments, an ideal switch's controlled/natural commutation modes — so
within any *fixed* combination of active segments the whole circuit is exactly linear. Which
segment each device is in, once per timestep, is resolved by solving a Linear Complementarity
Problem (LCP) via Lemke's algorithm, the same rigorous mode-selection approach commercial
power-electronics simulators use, instead of continuous Newton iteration on an exponential diode
curve. The companion Developer Guide's ["Why not
Newton-Raphson"](../dev-guide/architecture-overview.md) chapter has the full argument, including
where SPICE's own voltage limiting breaks down; this guide doesn't re-derive it.

## Who this guide is for

This is the **User Guide** — wiring up netlists, reading the output, running the CLI. If you're
extending the simulator itself (a new device model, a new block kind, the LCP formulation, the
adaptive-step controller), the companion **Developer Guide** is where the math and the "why"
live — this guide deliberately stays at the netlist-author's altitude.

## Why this instead of ngspice or Xyce

Two honest reasons, not a claim of general superiority: this project exists to sidestep voltage
limiting's own well-documented convergence quirks (inconsistent, path-dependent under
backtracking — a known wart even by Xyce's own maintainers' account) for the specific case of
piecewise-linear power-electronics devices, and to let a controller (PID, state-space, a PWM
modulator, coordinate transforms) be declared as ordinary netlist statements wired directly to
the circuit, rather than as a separate control-system tool bolted on afterward. If your circuit
genuinely needs BJT/MOSFET-level nonlinear device physics, this project doesn't model that yet —
reach for a real SPICE tool instead.
