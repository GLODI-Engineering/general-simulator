# Summary

[Introduction](introduction.md)

# Getting started

- [Installing and building](getting-started.md)
- [Your first netlist](first-netlist.md)
- [Reading the output](reading-output.md)

# The netlist and device-file grammar

- [Grammar overview](netlist-grammar.md)
- [PWL devices: diodes and ideal switches](pwl-devices.md)
- [Gate bindings (fixed, PWM, block-driven)](gate-bindings.md)
- [Signals: `meas:`, `prev:`, and same-step references](signals.md)

# The continuous-block library

- [Block library overview](block-library.md)
- [Component Reference](component-reference.md)
- [Sources and math operations](sources-and-math.md)
- [Dynamic blocks (PID, state-space, transfer function)](dynamic-blocks.md)
- [Coordinate transforms (Clarke/Park) and PLL](coordinate-transforms.md)
- [The PMSM block](pmsm.md)
- [The CScript escape hatch](cscript.md)

# CLI reference

- [Command-line flags](cli-reference.md)
- [Fixed vs. adaptive time stepping](time-stepping.md)

# Post-processing measurements

- [`kind=measure`: ngspice/Xyce-style .measure statements](measurements.md)

# Worked examples

- [Buck converter (open-loop and PID)](examples/buck.md)
- [Boost/LLC resonant converter](examples/boost-llc.md)
- [Three-phase PFC active front end](examples/pfc.md)
- [PMSM field-oriented-control drive](examples/pmsm-drive.md)

# Reference

- [Validation against ngspice and Xyce](validation.md)
- [Troubleshooting and gotchas](gotchas.md)
- [FAQ](faq.md)
