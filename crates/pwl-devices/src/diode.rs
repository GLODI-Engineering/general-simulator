//! A 3-segment piecewise-linear diode:
//!
//! ```text
//!            g_breakdown          g_off (leakage)         g_on (forward)
//!   <---------------------|------------------------|-------------------->
//!                     v_breakdown                 v_th
//! ```
//!
//! Continuous at both breakpoints by construction (each segment's offset is chosen so it
//! meets its neighbor exactly at the shared breakpoint).

/// A piecewise-linear diode: reverse-breakdown conduction below `v_breakdown`, near-zero
/// leakage between `v_breakdown` and `v_th`, forward conduction above `v_th`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Diode {
    pub g_breakdown: f64,
    pub v_breakdown: f64,
    pub g_off: f64,
    pub v_th: f64,
    pub g_on: f64,
}

impl Diode {
    pub fn new(g_breakdown: f64, v_breakdown: f64, g_off: f64, v_th: f64, g_on: f64) -> Self {
        assert!(
            v_breakdown < v_th,
            "v_breakdown ({v_breakdown}) must be strictly below v_th ({v_th})"
        );
        Diode {
            g_breakdown,
            v_breakdown,
            g_off,
            v_th,
            g_on,
        }
    }

    /// Direct piecewise evaluation — which segment `v` falls in, evaluated from that segment's
    /// own slope and its continuity anchor at the boundary it shares with the leakage segment.
    /// This is an independent formula from [`Diode::canonical`]'s decomposition, used to
    /// cross-check that decomposition rather than trusting it by construction.
    pub fn current(&self, v: f64) -> f64 {
        if v <= self.v_breakdown {
            let anchor = self.g_off * self.v_breakdown;
            anchor + self.g_breakdown * (v - self.v_breakdown)
        } else if v < self.v_th {
            self.g_off * v
        } else {
            let anchor = self.g_off * self.v_th;
            anchor + self.g_on * (v - self.v_th)
        }
    }

    /// The Chua-Lin canonical piecewise-linear decomposition of this diode's I-V curve,
    /// anchored on the leakage segment (which passes through the origin at `g_off` slope):
    ///
    /// ```text
    /// I(v) = g_off * v + delta_on * z2 - delta_br * z1
    /// z1 = max(0, v_breakdown - v)   (how far below breakdown, else 0)
    /// z2 = max(0, v - v_th)          (how far above threshold, else 0)
    /// ```
    ///
    /// `z1` and `z2` are exactly the LCP `z` variables this diode contributes: each is
    /// complementary to a slack `w1 = v - v_breakdown + z1`, `w2 = v_th - v + z2` (both >= 0,
    /// each `w_i * z_i == 0`). See `docs/architecture.md` for the derivation and why this is
    /// equivalent to "exactly one segment active."
    pub fn canonical(&self) -> DiodeCanonical {
        DiodeCanonical {
            v_breakdown: self.v_breakdown,
            v_th: self.v_th,
            g_off: self.g_off,
            delta_br: self.g_breakdown - self.g_off,
            delta_on: self.g_on - self.g_off,
        }
    }
}

impl Diode {
    /// The same physical diode with its two terminals swapped: `reversed().current(v) ==
    /// -self.current(-v)` for every `v`, i.e. conducts (forward) for `v < -self.v_th` instead
    /// of `v > self.v_th`, with breakdown now on the *positive* side instead of the negative
    /// one. Exists so a two-terminal device whose curve is defined in terms of `V = V(n1) -
    /// V(n2)` (every PWL device in this crate, including [`crate::Mosfet`]'s body diode) can be
    /// stamped correctly regardless of which physical terminal a caller happened to declare
    /// first — swap the *curve*, not the netlist nodes, when the caller's own terminal-order
    /// convention (e.g. `(drain, source)`) is the mirror image of this struct's own `V = anode
    /// - cathode` convention.
    ///
    /// Derivation: substituting `u = -v` into each of the three segments and negating gives a
    /// new 3-segment curve with the same shape (breakdown / leakage / forward, continuous at
    /// both breakpoints) but mirrored breakpoints and swapped slopes — which is exactly what
    /// `Diode::new` already builds, just called with `(g_on, -v_th, g_off, -v_breakdown,
    /// g_breakdown)` instead of the original `(g_breakdown, v_breakdown, g_off, v_th, g_on)`.
    /// `v_breakdown < v_th` (this struct's own invariant) guarantees `-v_th < -v_breakdown`, so
    /// the reversed curve's own breakpoint ordering is automatically valid too.
    pub fn reversed(&self) -> Self {
        Diode::new(
            self.g_on,
            -self.v_th,
            self.g_off,
            -self.v_breakdown,
            self.g_breakdown,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiodeCanonical {
    pub v_breakdown: f64,
    pub v_th: f64,
    pub g_off: f64,
    pub delta_br: f64,
    pub delta_on: f64,
}

impl DiodeCanonical {
    /// `z1 = max(0, v_breakdown - v)`, `z2 = max(0, v - v_th)` — the values Lemke's algorithm
    /// would resolve them to at a given `v`. Provided for testing/reference, not used by the
    /// LCP solve itself (which determines `v` and the `z`s simultaneously).
    pub fn z_at(&self, v: f64) -> (f64, f64) {
        ((self.v_breakdown - v).max(0.0), (v - self.v_th).max(0.0))
    }

    pub fn current(&self, v: f64, z1: f64, z2: f64) -> f64 {
        self.g_off * v + self.delta_on * z2 - self.delta_br * z1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canonical (z1, z2) decomposition and the direct piecewise formula are two
    /// independently written expressions for the same curve; agreeing at many points in every
    /// segment (including exactly at both breakpoints) is a real cross-check, not a tautology.
    #[test]
    fn canonical_matches_direct_piecewise_evaluation() {
        let d = Diode::new(50.0, -20.0, 1e-6, 0.7, 10.0);
        let c = d.canonical();
        let sample_voltages = [
            -30.0, -20.5, -20.0, -19.5, -10.0, -1.0, 0.0, 0.3, 0.6999, 0.7, 0.70001, 1.0, 5.0, 50.0,
        ];
        for &v in &sample_voltages {
            let direct = d.current(v);
            let (z1, z2) = c.z_at(v);
            let canonical = c.current(v, z1, z2);
            assert!(
                (direct - canonical).abs() < 1e-9,
                "mismatch at v={v}: direct={direct}, canonical={canonical}"
            );
        }
    }

    /// At the breakdown breakpoint the leakage and breakdown segments must agree (both equal
    /// the leakage segment's value there); same at the threshold breakpoint with the forward
    /// segment. This is the continuity the canonical construction is supposed to guarantee.
    #[test]
    fn continuous_at_both_breakpoints() {
        let d = Diode::new(50.0, -20.0, 1e-6, 0.7, 10.0);
        let eps = 1e-6;
        assert!((d.current(d.v_breakdown - eps) - d.current(d.v_breakdown + eps)).abs() < 1e-3);
        assert!((d.current(d.v_th - eps) - d.current(d.v_th + eps)).abs() < 1e-3);
    }

    #[test]
    #[should_panic(expected = "must be strictly below")]
    fn rejects_breakdown_at_or_above_threshold() {
        Diode::new(1.0, 1.0, 1e-6, 1.0, 1.0);
    }

    /// `reversed()`'s defining algebraic identity, sampled across every segment (including
    /// both breakpoints) of an asymmetric diode -- not just a few hand-picked points that
    /// happen to work.
    #[test]
    fn reversed_satisfies_its_own_defining_identity() {
        let d = Diode::new(50.0, -20.0, 1e-6, 0.7, 10.0);
        let r = d.reversed();
        for v in [
            -30.0, -20.0, -19.5, -1.0, 0.0, 0.5, 0.7, 0.70001, 1.0, 5.0, 50.0,
        ] {
            assert!(
                (r.current(v) - (-d.current(-v))).abs() < 1e-9,
                "mismatch at v={v}: reversed.current(v)={}, -d.current(-v)={}",
                r.current(v),
                -d.current(-v)
            );
        }
    }

    /// Concrete physical check, matching this session's own standalone CLI body-diode test on
    /// the TIDA-010954 controller's own MOSFET parameters (`g_breakdown=0, v_breakdown=-1e6,
    /// g_off=1e-6, v_th=0.6, g_on=100`): a diode reversed this way and then evaluated with `v =
    /// V(drain) - V(source)` (the plain SPICE-conventional node order, *not* this crate's
    /// `(source, drain)` convention) blocks for `v > 0` (near-zero leakage current) and
    /// conducts strongly for `v` well below `-v_th` -- the correct low-side-switch body-diode
    /// behavior without requiring the netlist to declare nodes in any unusual order.
    #[test]
    fn reversed_gives_correct_low_side_switch_behavior_at_spice_conventional_node_order() {
        let body_diode = Diode::new(0.0, -1e6, 1e-6, 0.6, 100.0);
        let r = body_diode.reversed();
        // v = V(drain) - V(source) = +10: drain driven positive relative to source -- a real
        // low-side NMOS's body diode blocks here (only tiny leakage).
        assert!(r.current(10.0).abs() < 1e-4);
        // v = -10: drain pulled well below source (below -v_th) -- the body diode conducts.
        assert!(r.current(-10.0) < -100.0);
    }
}
