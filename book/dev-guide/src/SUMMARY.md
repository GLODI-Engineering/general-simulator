# Summary

[Introduction](introduction.md)

# Architecture

- [Why not Newton-Raphson](architecture-overview.md)
- [The generic Thevenin/LCP fold](lcp-formulation.md)
- [DAE integration: backward Euler and trapezoidal](dae-integration.md)
- [Ringing detection and the backward-Euler fallback](ringing-and-fallback.md)

# Switching

- [Diodes and MOSFET body diodes: the endogenous case](switch-model-diodes.md)
- [MOSFET channel state: the exogenous case, and why](switch-model-mosfets.md)

# The block graph

- [Blocks as a descriptor-DAE fragment](block-graph-descriptor.md)
- [Computational causality: topological_order](block-graph-causality.md)
- [Algebraic loop detection](block-graph-cycles.md)
- [Dynamic blocks: one RK4 per block, wired by causality](block-graph-rk4.md)

# Crate tour

- [Workspace layout](crate-tour.md)
- [`lcp-solver`: Lemke's algorithm](crate-lcp-solver.md)
- [`pwl-devices`: diode and MOSFET models](crate-pwl-devices.md)
- [`dae-runtime`: circuit assembly and the transient loop](crate-dae-runtime.md)
- [`continuous-blocks`: the block library](crate-continuous-blocks.md)
- [`cscript-ffi`: the native escape hatch](crate-cscript-ffi.md)
- [`general-simulator-cli`: the netlist-in/CSV-out runner](crate-general-simulator-cli.md)

# Contributing

- [Verification discipline](verification-discipline.md)
- [Required gates and workflow](workflow.md)
- [Journal and gotchas conventions](journal-and-gotchas.md)
- [Project boundaries: the sibling repos](project-boundaries.md)

# Design rationale and open questions

- [Design decisions log](design-decisions.md)
- [Open extension points](open-questions.md)
- [Vector signals: per-block survey](vector-signals.md)
