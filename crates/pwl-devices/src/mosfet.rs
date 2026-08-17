use crate::Diode;

/// An idealized power MOSFET: a controlled switch when gated on, falling back to its
/// intrinsic body diode when gated off.
///
/// - **Gated on**: the channel conducts in *either* direction — a real MOSFET's channel is a
///   resistor, not a one-way valve — so this is just `r_on` between drain and source,
///   regardless of the sign of `V` or `I`. This is the "controlled commutation when `V > 0`
///   and `I > 0`" case from the spec: while on, the device actively forces low-impedance
///   conduction in the normal blocking direction too.
/// - **Gated off**: the channel is open, but the body diode (anode at the source, cathode at
///   the drain, for an N-channel device) still conducts if the circuit pushes current
///   backward through it — this is the "for `I < 0`, commutation is natural or driven by the
///   circuit" case: nothing *controls* this conduction, it just happens if the voltage across
///   the body diode exceeds its forward threshold.
///
/// Which of the two applies at a given timestep is **not** something an LCP resolves — the
/// gate command is an external, exogenously-known control input (from a PWM/controller), not
/// a function of circuit state. So a `Mosfet` isn't itself a single PWL curve the way a
/// [`Diode`] is: the caller (`dae-runtime`) picks one of the two representations up front,
/// per instance, per timestep, based on the known gate state — see `dae-runtime`'s
/// `solve_dc_with_mosfets`.
///
/// **Node-order convention**: to get the body diode's polarity right by reusing [`Diode`]
/// unchanged (forward conduction for `V > v_th`, `V` = first terminal minus second), a
/// `Mosfet` instance's two circuit terminals must be declared **`(source, drain)`**, not the
/// datasheet-conventional `(drain, source)`. A plain resistor (the gated-on case) doesn't
/// care about terminal order, so this convention costs nothing there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mosfet {
    /// On-state channel resistance (drain-source), conducting both directions.
    pub r_on: f64,
    /// The intrinsic body diode, evaluated at `V = V_source - V_drain` per the node-order
    /// convention above. Its `v_breakdown` segment is typically irrelevant (set far away,
    /// e.g. very negative) unless avalanche breakdown of the body diode itself matters to the
    /// circuit being modeled.
    pub body_diode: Diode,
}

impl Mosfet {
    pub fn new(r_on: f64, body_diode: Diode) -> Self {
        Mosfet { r_on, body_diode }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_diode_blocks_below_threshold_and_conducts_above() {
        let m = Mosfet::new(0.05, Diode::new(0.0, -1e4, 0.0, 0.7, 1.0));
        assert_eq!(m.body_diode.current(0.0), 0.0);
        assert_eq!(m.body_diode.current(0.5), 0.0);
        assert!((m.body_diode.current(1.7) - 1.0).abs() < 1e-9);
    }
}
