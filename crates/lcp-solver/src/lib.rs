//! Lemke's algorithm for the Linear Complementarity Problem (LCP).
//!
//! Given a matrix `M` (n x n) and a vector `q` (n), find `w, z in R^n` such that
//!
//! ```text
//! w = M z + q
//! w >= 0
//! z >= 0
//! w . z = 0        (complementary: for every i, w_i == 0 or z_i == 0)
//! ```
//!
//! This is the numerical primitive `elspice-pwl` uses to decide, once per timestep, which
//! linear segment of each piecewise-linear device (diode, MOSFET) is active — replacing
//! Newton-Raphson + voltage limiting entirely. See `docs/architecture.md` in this repository
//! for how device segments map onto `(M, q)`.
//!
//! This crate has no knowledge of circuits, devices, or `elspice-mna`; it only implements the
//! LCP numerics, so it can be tested and trusted in isolation first (see `tests/fixtures.rs`
//! for hand-solved textbook cases).

mod lemke;
mod tableau;

pub use lemke::{solve, LcpError, Solution};
