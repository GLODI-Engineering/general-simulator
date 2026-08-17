//! Continuous-time block-diagram elements — transfer function, state-space, PID, integrator,
//! filtered derivative, and stateless math ops — matching the standard `Continuous` and
//! `Math Operations` block sets common to block-diagram simulation tools.
//!
//! Every dynamic block compiles to a [`StateSpace`] — the same descriptor-DAE shape
//! `elspice-mna` already uses for circuits (`A x + K dx/dt = B u`, with `K` here called `e`
//! for descriptor systems). See `docs/architecture.md`, "One descriptor system for circuit
//! and continuous blocks alike," for why this is the organizing idea of the whole
//! `elspice-pwl` project, not just this crate.
//!
//! This crate is deliberately standalone (no dependency on `elspice-mna`, `pwl-devices`, or
//! `dae-runtime`): block parameters (gains, pole/zero locations, PID coefficients) are known
//! numbers at model-build time, not symbolic netlist parameters, and every block here is
//! verified against a hand-derived result on its own before anything wires it into a whole
//! circuit's global system — the same incremental-verification discipline `lcp-solver` and
//! `pwl-devices` were built with. Wiring a compiled block's `(A, K, B)` into `dae-runtime`'s
//! global descriptor system, and stateless [`math_ops`] blocks into its per-timestep
//! assembly, is a later milestone.

pub mod dynamics;
pub mod math_ops;
mod pid;
mod state_space;
mod transfer_function;

pub use dynamics::{derivative_filtered, integrator};
pub use pid::Pid;
pub use state_space::{SingularMatrix, StateSpace};
pub use transfer_function::{TransferFunction, TransferFunctionError};
