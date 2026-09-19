# continuous-blocks

Block-diagram elements for control and signal processing: transfer function, state-space, PID (continuous and discrete), VCO, hysteresis, PMSM, Clarke/Park coordinate transforms, logic, and stateless math operations. Dynamic blocks compile into descriptor-DAE fragments of the same `A x + K dx/dt = B u` shape used for circuits.

Standalone: it does not depend on the circuit or solver crates.

## Part of general-simulator

This crate is one piece of [general-simulator](https://github.com/GLODI-Engineering/general-simulator), an open-source Rust simulator for
piecewise-linear circuits and mixed circuit/block-diagram systems. Instead of Newton-Raphson with
voltage limiting, it resolves which linear segment every device is in as a Linear Complementarity
Problem each timestep. Documentation lives in the repository's `book/` directory.

## License

AGPL-3.0-or-later. See [LICENSE](https://github.com/GLODI-Engineering/general-simulator/blob/main/LICENSE).
