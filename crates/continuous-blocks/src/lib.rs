//! Continuous-time block-diagram elements — transfer function, state-space, PID,
//! voltage-controlled oscillator, hysteresis (Schmitt-trigger) comparator, and stateless math
//! ops (gain, sum, product, saturation) — matching the standard `Continuous`/`Math Operations`
//! block sets common to block-diagram simulation tools. Each block does one
//! job and is meant to be wired to the others by the caller — an error signal is a `Sum`
//! block's output, not something a controller computes internally, and a frequency-modulated
//! PWM carrier is `Pid -> Gain -> Vco`, not a single fused "closed loop" function. See
//! `dae-runtime`'s `closed_loop` module for how a circuit's MOSFET gates get wired to a chain
//! of these blocks.
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

pub mod coordinate_transforms;
mod hysteresis;
pub mod math_ops;
mod pid;
mod pmsm;
mod state_space;
mod transfer_function;
mod vco;
pub mod waveform_arithmetic;

pub use coordinate_transforms::CoordinateTransform;
pub use hysteresis::{Hysteresis, HysteresisError};
pub use pid::{Pid, PidError};
pub use pmsm::{Pmsm, PmsmError};
pub use state_space::{StateSpace, StateSpaceError};
pub use transfer_function::{TransferFunction, TransferFunctionError};
pub use vco::{Vco, VcoError};
pub use waveform_arithmetic::{MathFn1, MathFn2, MathFn3};
