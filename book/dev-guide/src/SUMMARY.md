# Summary

[Introduction](introduction.md)

# Architecture

- [Why not Newton-Raphson](architecture-overview.md)
- [The generic Thevenin/LCP fold](lcp-formulation.md)
- [DAE integration: backward Euler and trapezoidal](dae-integration.md)
- [Ringing detection and the backward-Euler fallback](ringing-and-fallback.md)

# Switching

- [Diodes and ideal-switch body diodes: the endogenous case](switch-model-diodes.md)
- [Ideal-switch channel state: the exogenous case, and why](switch-model-ideal-switch.md)

# The block graph

- [Blocks as a descriptor-DAE fragment](block-graph-descriptor.md)
- [Computational causality: topological_order](block-graph-causality.md)
- [Algebraic loop detection](block-graph-cycles.md)
- [Dynamic blocks: one RK4 per block, wired by causality](block-graph-rk4.md)

# Crate tour

- [Workspace layout](crate-tour.md)
- [`lcp-solver`: Lemke's algorithm](crate-lcp-solver.md)
- [`pwl-devices`: ideal-diode and ideal-switch models](crate-pwl-devices.md)
- [`dae-runtime`: circuit assembly and the transient loop](crate-dae-runtime.md)
- [`continuous-blocks`: the block library](crate-continuous-blocks.md)
- [`cscript-ffi`: the native escape hatch](crate-cscript-ffi.md)
- [`pyblock-ffi`: the Python escape hatch](python-blocks.md)
- [`pyblock-ffi`: pure-function blocks (`kind=pyfunc`)](pyfunc-blocks.md)
- [`general-simulator-cli`: the netlist-in/CSV-out runner](crate-general-simulator-cli.md)
- [`kind=measure`: post-processing measurements, not a block-graph kind](measurements-architecture.md)

# Contributing

- [Verification discipline](verification-discipline.md)
- [Required gates and workflow](workflow.md)
- [Journal and gotchas conventions](journal-and-gotchas.md)
- [Project boundaries: the sibling repos](project-boundaries.md)

# Design rationale and open questions

- [Design decisions log](design-decisions.md)
- [Open extension points](open-questions.md)
- [Vector signals: per-block survey](vector-signals.md)
- [Logic signals: gates, latches, flip-flops, counters](logic-signals.md)
- [Discrete-time blocks: state-space, transfer function, PID](discrete-time-blocks.md)
