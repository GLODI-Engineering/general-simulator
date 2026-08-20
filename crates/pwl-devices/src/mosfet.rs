use crate::Diode;

/// An idealized power MOSFET: a controlled switch when gated on, falling back to its
/// intrinsic body diode when gated off.
///
/// **The real physics this models** (confirmed against a real MOSFET's own behavior, not
/// assumed, after an earlier version of this doc comment described it slightly wrong):
///
/// - Channel on/off depends only on `Vgs` vs `Vth`, not on the sign of `Vds`. A real MOSFET's
///   channel, once on, conducts in both directions almost symmetrically (that's literally what
///   synchronous rectification exploits — the channel carries the freewheeling current at a
///   much lower drop than the body diode would, for `Vds<0` too, as long as the gate is still
///   on). So it's not "`Vds>0` → switch behavior, `Vds<0` → diode behavior" as two
///   `Vds`-conditioned regimes — it's "gate on → switch behavior for *any* `Vds`," and "gate
///   off → body-diode-only behavior for *any* `Vds`" (blocking when `Vds>0`, conducting when
///   `Vds<−Vf`). `Vds`'s sign only matters *given* the gate is off.
/// - **Gated on**: the channel conducts in *either* direction — a real MOSFET's channel is a
///   resistor, not a one-way valve — so this is just `r_on` between drain and source,
///   regardless of the sign of `Vds`.
/// - **Gated off**: the channel is open, but the body diode (anode at the source, cathode at
///   the drain, for an N-channel device) still conducts if the circuit pushes current backward
///   through it, independent of the (now-irrelevant, since the channel is off) gate command.
///
/// Which of the two applies at a given timestep is **not** something an LCP resolves — the
/// gate command is an external, exogenously-known control input (from a PWM/controller), not a
/// function of circuit state, and not a modeled `Vgs` compared against a threshold anywhere in
/// this crate: the caller (`dae-runtime`) already knows the gate state before the circuit is
/// even built, and picks one of the two representations up front, per instance, per timestep —
/// see `dae-runtime`'s `solve_dc_with_mosfets`.
///
/// **Node-order convention: plain SPICE-conventional `(drain, source)`.** `body_diode` is
/// declared the ordinary way a diode is (forward conduction — from anode to cathode — for `V =
/// anode - cathode > v_th`), with the physical anode at the MOSFET's own source and cathode at
/// its drain — the same body-diode orientation every real N-channel MOSFET has. Since the
/// netlist declares this device's two terminals `(drain, source)`, not `(source, drain)`, the
/// diode as literally stamped between those two nodes needs its curve mirrored first — see
/// [`Mosfet::body_diode_for_drain_source_stamping`], which every caller stamping this device's
/// off-state should use instead of `body_diode` directly. (An earlier version of this struct
/// required the netlist to declare `(source, drain)` instead, so the *unmirrored* `body_diode`
/// stamped correctly as-is — that convention was a real, confirmed footgun: 6 of 8 MOSFETs in
/// the TIDA-010954 cycloconverter experiment were declared in the natural, SPICE-conventional
/// `(drain, source)` order and so had their body diodes backwards. This struct's contract
/// changed specifically to remove that footgun, not just document it better.)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mosfet {
    /// On-state channel resistance (drain-source), conducting both directions.
    pub r_on: f64,
    /// The intrinsic body diode, in its own natural terms (anode at the source, cathode at the
    /// drain — forward conduction for `V(source) - V(drain) > v_th`). Do **not** stamp this
    /// directly against a `(drain, source)`-ordered netlist declaration — use
    /// [`Mosfet::body_diode_for_drain_source_stamping`] instead, which returns the curve
    /// already mirrored for that node order.
    pub body_diode: Diode,
}

impl Mosfet {
    pub fn new(r_on: f64, body_diode: Diode) -> Self {
        Mosfet { r_on, body_diode }
    }

    /// The body diode's curve, mirrored so that evaluating it with `V = V(drain) - V(source)`
    /// (the plain SPICE-conventional node order this crate's netlists declare a MOSFET's two
    /// terminals in) gives the physically correct result: blocks for `V > 0`, conducts for `V
    /// < -v_th`. See [`Diode::reversed`] for the derivation, and this struct's own doc comment
    /// for why this indirection exists at all instead of just documenting a required node
    /// order.
    pub fn body_diode_for_drain_source_stamping(&self) -> Diode {
        self.body_diode.reversed()
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

    /// The stamping-oriented curve should block at a positive drain-source voltage and conduct
    /// at a sufficiently negative one -- the correct behavior for a device declared `(drain,
    /// source)`, matching this session's own standalone CLI verification.
    #[test]
    fn body_diode_for_drain_source_stamping_matches_plain_spice_node_order() {
        let m = Mosfet::new(0.05, Diode::new(0.0, -1e6, 1e-6, 0.6, 100.0));
        let stamped = m.body_diode_for_drain_source_stamping();
        assert!(stamped.current(10.0).abs() < 1e-4);
        assert!(stamped.current(-10.0) < -100.0);
    }
}
