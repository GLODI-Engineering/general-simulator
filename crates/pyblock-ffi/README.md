# pyblock-ffi

Embeds a Python interpreter (via PyO3) and calls user-supplied Python functions once per simulation step: stateful blocks (`start`/`output`/`update`) and stateless function blocks.

Requires a Python installation. The runner crate enables it with the `python` Cargo feature.

## Part of general-simulator

This crate is one piece of [general-simulator](https://github.com/GLODI-Engineering/general-simulator), an open-source Rust simulator for
piecewise-linear circuits and mixed circuit/block-diagram systems. Instead of Newton-Raphson with
voltage limiting, it resolves which linear segment every device is in as a Linear Complementarity
Problem each timestep. Documentation lives in the repository's `book/` directory.

## License

AGPL-3.0-or-later. See [LICENSE](https://github.com/GLODI-Engineering/general-simulator/blob/main/LICENSE).
