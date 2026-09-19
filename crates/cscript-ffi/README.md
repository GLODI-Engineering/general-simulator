# cscript-ffi

Loads and calls user-supplied, dynamically linked C blocks, once per simulation step, so native control code can be tested against a simulated power stage.

This is the one place in the workspace where unsafe FFI into arbitrary native code is deliberately allowed, and it is kept isolated from every other crate.

## Part of general-simulator

This crate is one piece of [general-simulator](https://github.com/GLODI-Engineering/general-simulator), an open-source Rust simulator for
piecewise-linear circuits and mixed circuit/block-diagram systems. Instead of Newton-Raphson with
voltage limiting, it resolves which linear segment every device is in as a Linear Complementarity
Problem each timestep. Documentation lives in the repository's `book/` directory.

## License

AGPL-3.0-or-later. See [LICENSE](https://github.com/GLODI-Engineering/general-simulator/blob/main/LICENSE).
