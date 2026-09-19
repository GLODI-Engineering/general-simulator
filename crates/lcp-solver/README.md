# lcp-solver

Lemke's algorithm for the Linear Complementarity Problem (LCP): given `M` and `q`, find `w, z >= 0` with `w = Mz + q` and `w_i z_i = 0`. Used by general-simulator to pick the active segment of every piecewise-linear device each timestep.

Pure numerics with no knowledge of circuits, so it can be tested in isolation against hand-solved fixtures. Returns clean errors (`RayTermination`, `MaxIterationsExceeded`, `NonFiniteInput`) instead of panicking on degenerate or non-finite input.

## Part of general-simulator

This crate is one piece of [general-simulator](https://github.com/GLODI-Engineering/general-simulator), an open-source Rust simulator for
piecewise-linear circuits and mixed circuit/block-diagram systems. Instead of Newton-Raphson with
voltage limiting, it resolves which linear segment every device is in as a Linear Complementarity
Problem each timestep. Documentation lives in the repository's `book/` directory.

## License

AGPL-3.0-or-later. See [LICENSE](https://github.com/GLODI-Engineering/general-simulator/blob/main/LICENSE).
