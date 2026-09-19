# dae-runtime

Circuit assembly and the transient loop. Combines a linear MNA system (from `general-mna`) with piecewise-linear device segments (from `pwl-devices`) into one LCP, solves it with `lcp-solver`, and steps in time with backward Euler or trapezoidal integration, fixed or adaptive step, plus the block-graph evaluator that wires control blocks to the circuit.

No Newton-Raphson and no voltage limiting anywhere.

## Part of general-simulator

This crate is one piece of [general-simulator](https://github.com/GLODI-Engineering/general-simulator), an open-source Rust simulator for
piecewise-linear circuits and mixed circuit/block-diagram systems. Instead of Newton-Raphson with
voltage limiting, it resolves which linear segment every device is in as a Linear Complementarity
Problem each timestep. Documentation lives in the repository's `book/` directory.

## License

AGPL-3.0-or-later. See [LICENSE](https://github.com/GLODI-Engineering/general-simulator/blob/main/LICENSE).
