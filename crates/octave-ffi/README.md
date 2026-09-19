# octave-ffi

Drives a persistent `octave-cli` subprocess to call user-supplied Octave `.m` functions once per simulation step.

Octave is never linked: it runs as a separate process, so there is no build-time dependency. `octave-cli` must be on `PATH` at run time.

## Part of general-simulator

This crate is one piece of [general-simulator](https://github.com/GLODI-Engineering/general-simulator), an open-source Rust simulator for
piecewise-linear circuits and mixed circuit/block-diagram systems. Instead of Newton-Raphson with
voltage limiting, it resolves which linear segment every device is in as a Linear Complementarity
Problem each timestep. Documentation lives in the repository's `book/` directory.

## License

AGPL-3.0-or-later. See [LICENSE](https://github.com/GLODI-Engineering/general-simulator/blob/main/LICENSE).
