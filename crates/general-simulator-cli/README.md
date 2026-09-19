# general-simulator-cli

The `general-simulator` command-line runner: a SPICE-style netlist (with `kind=` block and
device lines) in, a waveform out as CSV or a SPICE raw file.

## Install

```bash
cargo install general-simulator-cli
```

To enable the Python blocks (`kind=pyblock` / `kind=pyfunc`), install with the `python` feature:

```bash
cargo install general-simulator-cli --features python
```

## Use

```bash
general-simulator netlist.cir --mode transient --tfinal 2e-3 --dt 1e-7 > out.csv
general-simulator netlist.cir --mode dc
```

Flags: `--mode dc|transient`, `--tfinal`, `--dt`, `--format csv|raw`, `--out <path>`, `--out-every N`. CSV goes to stdout by default. Netlist grammar, the full component reference,
worked examples (buck, boost/LLC, PMSM drive), and validation against other simulators are in the
documentation under the repository's `book/` directory.

## Features

- Piecewise-linear diodes and ideal switches resolved as an LCP, with no Newton-Raphson.
- Native control blocks in the netlist: transfer function, state-space, PID, coordinate transforms.
- Fixed or adaptive time stepping, decimated output, CSV or SPICE raw output.
- `kind=measure` post-processing (average, RMS, integral, crossings, and more).
- C, Python, and Octave escape hatches for testing real control code.

## Part of general-simulator

This crate is one piece of [general-simulator](https://github.com/GLODI-Engineering/general-simulator), an open-source Rust simulator for
piecewise-linear circuits and mixed circuit/block-diagram systems. Instead of Newton-Raphson with
voltage limiting, it resolves which linear segment every device is in as a Linear Complementarity
Problem each timestep. Documentation lives in the repository's `book/` directory.

## License

AGPL-3.0-or-later. See [LICENSE](https://github.com/GLODI-Engineering/general-simulator/blob/main/LICENSE).
