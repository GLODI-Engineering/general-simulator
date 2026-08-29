//! Piecewise-linear device companion models.
//!
//! Each device exposes a Chua-Lin canonical decomposition of its I-V curve into a reference
//! linear term plus `max(0, ...)` terms at each breakpoint. Those `max(0, ...)` terms are
//! exactly the `z` variables of a Linear Complementarity Problem (solved by `lcp-solver`) that
//! picks out, per timestep, which segment of every PWL device in a circuit is active — this is
//! what replaces Newton-Raphson + voltage limiting. See `docs/architecture.md` in this
//! repository for the full derivation.
//!
//! This crate only defines device *curves* (pure functions of a device's own terminal
//! voltage/current); folding a set of devices into a whole circuit's `(M, q)` LCP is
//! `dae-runtime`'s job (a later milestone). See `tests/two_ideal_diode_circuit.rs` for a
//! hand-worked example of that folding done manually, as the first end-to-end proof this
//! approach works.

mod ideal_diode;
mod ideal_switch;

pub use ideal_diode::{IdealDiode, IdealDiodeCanonical};
pub use ideal_switch::IdealSwitch;
