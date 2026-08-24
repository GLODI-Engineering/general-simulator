//! A standard real-valued scalar function library — trig, exponential/log, rounding, sign,
//! select/clamp, and table lookup — offered here as ordinary stateless blocks alongside
//! [`crate::math_ops`]'s `gain`/`sum`/`product`/`saturation`. This is the common vocabulary
//! most waveform-arithmetic/expression languages across circuit simulation tools converge on
//! (trig, exponential/log, sign, min/max, a select/if, a clamp, a lookup table), offered here
//! as composable blocks instead of a second expression language. Each function is a pure
//! function of its argument(s), evaluated fresh every step, exactly like the rest of
//! `math_ops`.
//!
//! Not included, on purpose:
//! - A finite-difference derivative, or random/noise generators — genuinely stateful or
//!   history-dependent, not stateless math, so they don't belong in this module (a derivative
//!   belongs in `dynamics`/an explicit `StateSpace`; a real PRNG/noise block would be a
//!   legitimate future addition, just a different one).
//! - Complex-data functions (real/imaginary part, phase, magnitude, conjugate) — this project
//!   has no complex-valued signals; everything here is a real-valued time-domain transient.
//! - Boolean/comparison *operators* (bitwise and/or/xor, greater/less-than, negation) — those
//!   belong to a plot-expression language for post-processing saved waveforms, not simulation
//!   blocks; `if`/`limit` below cover the actual control-logic use case (a comparator/select
//!   block) a real circuit's controller would need.
//!
//! [`MathFn1`]/[`MathFn2`]/[`MathFn3`] group the one/two/three-argument functions so
//! `dae-runtime`'s block graph (and `elspice-pwl-cli`'s device-file parser) can dispatch by
//! name without one bespoke `BlockKind` variant per function.

/// Sign of `x`: `-1`, `0`, or `1` — deliberately *not* `f64::signum` (which returns `1.0` for
/// `0.0`; this `sgn` returns `0` at the origin instead, the conventional signum definition).
pub fn sgn(x: f64) -> f64 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        0.0
    }
}

/// `abs(x)**y` — like [`pow`], but always real-valued (a negative base raised to a
/// non-integer power would otherwise be complex).
pub fn pwr(x: f64, y: f64) -> f64 {
    x.abs().powf(y)
}

/// `sgn(x) * abs(x)**y` — [`pwr`] with the original sign of `x` reapplied, an odd-symmetric
/// power function (stays real and odd-symmetric where a plain signed power would not).
pub fn pwrs(x: f64, y: f64) -> f64 {
    sgn(x) * x.abs().powf(y)
}

/// Unit step: `1` if `x > 0`, else `0`.
pub fn u(x: f64) -> f64 {
    if x > 0.0 {
        1.0
    } else {
        0.0
    }
}

/// `x` if `x > 0`, else `0` (a ramp that starts at the origin — the same function most
/// block-diagram tools call `Relu`/`Ramp`).
pub fn uramp(x: f64) -> f64 {
    if x > 0.0 {
        x
    } else {
        0.0
    }
}

/// `1` if `x > 0.5`, else `0` — a threshold/buffer, the companion to [`inv`].
pub fn buf(x: f64) -> f64 {
    if x > 0.5 {
        1.0
    } else {
        0.0
    }
}

/// `0` if `x > 0.5`, else `1` — the complement of [`buf`].
pub fn inv(x: f64) -> f64 {
    if x > 0.5 {
        0.0
    } else {
        1.0
    }
}

/// `y` if `x > 0.5`, else `z` — a select/comparator block (the control-logic use case a
/// separate Boolean expression language would otherwise cover, expressed here as an ordinary
/// three-input block instead).
pub fn if_then_else(x: f64, y: f64, z: f64) -> f64 {
    if x > 0.5 {
        y
    } else {
        z
    }
}

/// Clamps `x` to the range spanned by `y` and `z` (whichever of `y`/`z` is smaller is the
/// floor) — the "intermediate value of x, y, and z."
pub fn limit(x: f64, y: f64, z: f64) -> f64 {
    let (lo, hi) = if y <= z { (y, z) } else { (z, y) };
    x.clamp(lo, hi)
}

/// Linear interpolation through a set of `(x, y)` points (sorted by `x`, not necessarily
/// evenly spaced) — a `table(x, a, b, c, d, ...)`-style lookup. Clamped (not extrapolated)
/// outside the given range, holding the nearest endpoint's `y`. `points` must have at least
/// one entry and be sorted ascending by `x` (the caller's responsibility — a device-file-level
/// parser is the natural place to sort once at load time, not every step).
pub fn table(x: f64, points: &[(f64, f64)]) -> f64 {
    assert!(!points.is_empty(), "table needs at least one point");
    if x <= points[0].0 {
        return points[0].1;
    }
    let last = points.len() - 1;
    if x >= points[last].0 {
        return points[last].1;
    }
    let i = points.partition_point(|&(px, _)| px < x);
    let (x0, y0) = points[i - 1];
    let (x1, y1) = points[i];
    y0 + (y1 - y0) * (x - x0) / (x1 - x0)
}

/// One of the single-argument real waveform-arithmetic functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathFn1 {
    Abs,
    Acos,
    Acosh,
    Asin,
    Asinh,
    Atan,
    Atanh,
    Buf,
    Ceil,
    Cos,
    Cosh,
    Exp,
    Floor,
    Int,
    Inv,
    Ln,
    Log10,
    Round,
    Sgn,
    Sin,
    Sinh,
    Sqrt,
    Tan,
    Tanh,
    U,
    Uramp,
}

impl MathFn1 {
    /// Matches the conventional lowercase function name exactly (case-sensitive).
    pub fn from_name(name: &str) -> Option<Self> {
        use MathFn1::*;
        Some(match name {
            "abs" => Abs,
            "acos" => Acos,
            "acosh" => Acosh,
            "asin" => Asin,
            "asinh" => Asinh,
            "atan" => Atan,
            "atanh" => Atanh,
            "buf" => Buf,
            "ceil" => Ceil,
            "cos" => Cos,
            "cosh" => Cosh,
            "exp" => Exp,
            "floor" => Floor,
            "int" => Int,
            "inv" => Inv,
            "ln" => Ln,
            "log10" => Log10,
            "round" => Round,
            "sgn" => Sgn,
            "sin" => Sin,
            "sinh" => Sinh,
            "sqrt" => Sqrt,
            "tan" => Tan,
            "tanh" => Tanh,
            "u" => U,
            "uramp" => Uramp,
            _ => return None,
        })
    }

    /// The exact inverse of [`Self::from_name`] — the `kind=` keyword that selects this
    /// function at the CLI level. Used for diagnostics that need to name *this specific*
    /// function rather than the `MathFn1` family as a whole, e.g. `dae-runtime`'s own
    /// `block_kind_name`.
    pub fn name(self) -> &'static str {
        use MathFn1::*;
        match self {
            Abs => "abs",
            Acos => "acos",
            Acosh => "acosh",
            Asin => "asin",
            Asinh => "asinh",
            Atan => "atan",
            Atanh => "atanh",
            Buf => "buf",
            Ceil => "ceil",
            Cos => "cos",
            Cosh => "cosh",
            Exp => "exp",
            Floor => "floor",
            Int => "int",
            Inv => "inv",
            Ln => "ln",
            Log10 => "log10",
            Round => "round",
            Sgn => "sgn",
            Sin => "sin",
            Sinh => "sinh",
            Sqrt => "sqrt",
            Tan => "tan",
            Tanh => "tanh",
            U => "u",
            Uramp => "uramp",
        }
    }

    pub fn call(self, x: f64) -> f64 {
        use MathFn1::*;
        match self {
            Abs => x.abs(),
            Acos => x.acos(),
            Acosh => x.acosh(),
            Asin => x.asin(),
            Asinh => x.asinh(),
            Atan => x.atan(),
            Atanh => x.atanh(),
            Buf => buf(x),
            Ceil => x.ceil(),
            Cos => x.cos(),
            Cosh => x.cosh(),
            Exp => x.exp(),
            Floor => x.floor(),
            Int => x.trunc(),
            Inv => inv(x),
            Ln => x.ln(),
            Log10 => x.log10(),
            Round => x.round(),
            Sgn => sgn(x),
            Sin => x.sin(),
            Sinh => x.sinh(),
            Sqrt => x.sqrt(),
            Tan => x.tan(),
            Tanh => x.tanh(),
            U => u(x),
            Uramp => uramp(x),
        }
    }
}

/// One of the two-argument real waveform-arithmetic functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathFn2 {
    Atan2,
    /// `(alpha, beta) -> theta`, wrapped to `[0, 2*pi)` — see
    /// [`crate::coordinate_transforms::angle_wrapped`], the angle-tracking half of a
    /// synchronous-reference-frame PLL. Grouped here rather than with the other Clarke/Park
    /// transforms since, unlike those, it is single-output — an ordinary two-argument function
    /// like every other [`MathFn2`], just implemented in `coordinate_transforms` because it's
    /// conceptually paired with [`crate::coordinate_transforms::park`]/
    /// [`crate::coordinate_transforms::clarke_park`], which consume its output.
    AngleWrapped,
    Hypot,
    Max,
    Min,
    Pow,
    Pwr,
    Pwrs,
}

impl MathFn2 {
    pub fn from_name(name: &str) -> Option<Self> {
        use MathFn2::*;
        Some(match name {
            "atan2" => Atan2,
            "angle_wrapped" => AngleWrapped,
            "hypot" => Hypot,
            "max" => Max,
            "min" => Min,
            "pow" => Pow,
            "pwr" => Pwr,
            "pwrs" => Pwrs,
            _ => return None,
        })
    }

    /// The exact inverse of [`Self::from_name`] — see [`MathFn1::name`]'s own doc comment.
    pub fn name(self) -> &'static str {
        use MathFn2::*;
        match self {
            Atan2 => "atan2",
            AngleWrapped => "angle_wrapped",
            Hypot => "hypot",
            Max => "max",
            Min => "min",
            Pow => "pow",
            Pwr => "pwr",
            Pwrs => "pwrs",
        }
    }

    pub fn call(self, x: f64, y: f64) -> f64 {
        use MathFn2::*;
        match self {
            Atan2 => x.atan2(y),
            AngleWrapped => crate::coordinate_transforms::angle_wrapped(x, y),
            Hypot => x.hypot(y),
            Max => x.max(y),
            Min => x.min(y),
            Pow => x.powf(y),
            Pwr => pwr(x, y),
            Pwrs => pwrs(x, y),
        }
    }
}

/// One of the three-argument real waveform-arithmetic functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MathFn3 {
    If,
    Limit,
}

impl MathFn3 {
    pub fn from_name(name: &str) -> Option<Self> {
        use MathFn3::*;
        Some(match name {
            "if" => If,
            "limit" => Limit,
            _ => return None,
        })
    }

    /// The exact inverse of [`Self::from_name`] — see [`MathFn1::name`]'s own doc comment.
    pub fn name(self) -> &'static str {
        match self {
            MathFn3::If => "if",
            MathFn3::Limit => "limit",
        }
    }

    pub fn call(self, x: f64, y: f64, z: f64) -> f64 {
        match self {
            MathFn3::If => if_then_else(x, y, z),
            MathFn3::Limit => limit(x, y, z),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{E, PI};

    #[test]
    fn sgn_returns_zero_at_origin_unlike_f64_signum() {
        assert_eq!(sgn(5.0), 1.0);
        assert_eq!(sgn(-5.0), -1.0);
        assert_eq!(sgn(0.0), 0.0);
        // the whole reason this isn't just f64::signum:
        assert_eq!(0.0_f64.signum(), 1.0);
    }

    #[test]
    fn trig_and_exp_match_hand_known_values() {
        assert!((MathFn1::Cos.call(0.0) - 1.0).abs() < 1e-12);
        assert!((MathFn1::Sin.call(PI / 2.0) - 1.0).abs() < 1e-12);
        assert!((MathFn1::Exp.call(1.0) - E).abs() < 1e-12);
        assert!((MathFn1::Ln.call(E) - 1.0).abs() < 1e-12);
        assert!((MathFn1::Sqrt.call(4.0) - 2.0).abs() < 1e-12);
        assert!((MathFn1::Log10.call(100.0) - 2.0).abs() < 1e-12);
    }

    #[test]
    fn u_and_uramp_match_hand_derived_step_and_ramp() {
        assert_eq!(u(-1.0), 0.0);
        assert_eq!(u(1.0), 1.0);
        assert_eq!(uramp(-3.0), 0.0);
        assert_eq!(uramp(3.0), 3.0);
    }

    #[test]
    fn buf_and_inv_threshold_at_one_half() {
        assert_eq!(buf(0.4), 0.0);
        assert_eq!(buf(0.6), 1.0);
        assert_eq!(inv(0.4), 1.0);
        assert_eq!(inv(0.6), 0.0);
    }

    #[test]
    fn if_then_else_selects_by_threshold() {
        assert_eq!(if_then_else(1.0, 10.0, 20.0), 10.0);
        assert_eq!(if_then_else(0.0, 10.0, 20.0), 20.0);
    }

    #[test]
    fn limit_clamps_to_the_span_of_its_two_bounds_either_order() {
        assert_eq!(limit(5.0, 0.0, 2.0), 2.0);
        assert_eq!(limit(5.0, 2.0, 0.0), 2.0); // bounds given reversed
        assert_eq!(limit(1.0, 0.0, 2.0), 1.0);
    }

    #[test]
    fn pow_pwr_pwrs_hand_derived() {
        assert!((MathFn2::Pow.call(2.0, 3.0) - 8.0).abs() < 1e-12);
        // pwr(x,y) = abs(x)**y -- stays real for a negative base and non-integer exponent,
        // where a plain pow() would go complex.
        assert!((pwr(-8.0, 1.0 / 3.0) - 2.0).abs() < 1e-9);
        // pwrs reapplies the original sign on top of pwr.
        assert!((pwrs(-8.0, 1.0 / 3.0) - (-2.0)).abs() < 1e-9);
    }

    #[test]
    fn atan2_and_hypot_match_hand_derived_values() {
        // atan2(1,1) = 45 degrees = pi/4
        assert!((MathFn2::Atan2.call(1.0, 1.0) - PI / 4.0).abs() < 1e-12);
        // hypot(3,4) = 5, the textbook 3-4-5 triangle
        assert!((MathFn2::Hypot.call(3.0, 4.0) - 5.0).abs() < 1e-12);
    }

    #[test]
    fn angle_wrapped_dispatch_matches_direct_call() {
        assert_eq!(
            MathFn2::AngleWrapped.call(-1.0, -1.0),
            crate::coordinate_transforms::angle_wrapped(-1.0, -1.0)
        );
        assert_eq!(
            MathFn2::from_name("angle_wrapped"),
            Some(MathFn2::AngleWrapped)
        );
    }

    #[test]
    fn min_max_pick_the_right_side() {
        assert_eq!(MathFn2::Min.call(3.0, 7.0), 3.0);
        assert_eq!(MathFn2::Max.call(3.0, 7.0), 7.0);
    }

    #[test]
    fn table_interpolates_linearly_and_clamps_outside_range() {
        let points = [(0.0, 0.0), (1.0, 10.0), (2.0, 10.0), (3.0, 0.0)];
        assert_eq!(table(0.5, &points), 5.0); // halfway between (0,0) and (1,10)
        assert_eq!(table(1.5, &points), 10.0); // flat segment
        assert_eq!(table(-1.0, &points), 0.0); // clamped below range
        assert_eq!(table(10.0, &points), 0.0); // clamped above range
    }

    #[test]
    fn from_name_matches_conventional_names_and_rejects_unknown() {
        assert_eq!(MathFn1::from_name("cos"), Some(MathFn1::Cos));
        assert_eq!(MathFn1::from_name("nope"), None);
        assert_eq!(MathFn2::from_name("atan2"), Some(MathFn2::Atan2));
        assert_eq!(MathFn3::from_name("if"), Some(MathFn3::If));
    }
}
