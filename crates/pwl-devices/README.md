# pwl-devices

Piecewise-linear device models for LCP-based mode selection: a 3-segment ideal diode and an ideal switch (gated channel plus body diode), expressed in Chua-Lin canonical form.

This crate defines device curves only. Folding devices into a circuit's LCP is the job of `dae-runtime`.

## Part of general-simulator

This crate is one piece of [general-simulator](https://github.com/GLODI-Engineering/general-simulator), an open-source Rust simulator for
piecewise-linear circuits and mixed circuit/block-diagram systems. Instead of Newton-Raphson with
voltage limiting, it resolves which linear segment every device is in as a Linear Complementarity
Problem each timestep. Documentation lives in the repository's `book/` directory.

## License

AGPL-3.0-or-later. See [LICENSE](https://github.com/GLODI-Engineering/general-simulator/blob/main/LICENSE).
