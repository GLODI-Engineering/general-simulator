//! Clarke/Park coordinate transforms — the standard change of basis between a three-phase
//! `abc` signal, its stationary two-axis `alpha`/`beta` (plus zero-sequence) projection, and a
//! rotating `d`/`q` (plus zero-sequence) frame locked to a reference angle `theta`. Used to turn
//! a three-phase grid or motor-drive quantity into DC-like `d`/`q` values a `Pid` can regulate
//! directly, and the matching angle-tracking utility (`angle_wrapped`) that closes the loop
//! (a synchronous-reference-frame PLL is `Clarke -> angle_wrapped -> Park`, feeding a `Pid` on
//! the `q` output back into the tracked angle).
//!
//! Every function here is a pure, stateless function of its instantaneous inputs (no
//! integration, no memory) — the same category as [`crate::math_ops`] and
//! [`crate::waveform_arithmetic`], just multi-output. [`CoordinateTransform`] groups them the
//! same way [`crate::MathFn1`]/[`crate::MathFn2`] group the single-output functions, so
//! `dae-runtime`'s block graph can dispatch by one enum instead of one bespoke `BlockKind`
//! variant per transform.
//!
//! [`clarke_park`]/[`clarke_park_inv`] are convenience compositions of [`clarke`]/[`park`] (and
//! their inverses) directly against `abc`, skipping the `alpha`/`beta` intermediate — see
//! `clarke_park_matches_clarke_then_park` below for the equivalence this relies on. Amplitude
//! convention throughout is the standard "2/3" (non-power-invariant) scaling: a balanced
//! three-phase signal of peak amplitude `A` transforms to a `d`/`q` pair of magnitude `A`, not
//! `A * sqrt(3/2)`.

use std::f64::consts::PI;

const TWO_THIRDS: f64 = 2.0 / 3.0;

/// `abc -> (alpha, beta, zero)`.
pub fn clarke(a: f64, b: f64, c: f64) -> [f64; 3] {
    let alpha = TWO_THIRDS * (a - 0.5 * b - 0.5 * c);
    let beta = (b - c) / 3.0_f64.sqrt();
    let zero = (a + b + c) / 3.0;
    [alpha, beta, zero]
}

/// `(alpha, beta, zero) -> abc`. Exact inverse of [`clarke`].
pub fn clarke_inv(alpha: f64, beta: f64, zero: f64) -> [f64; 3] {
    let a = alpha + zero;
    let b = -0.5 * alpha + 0.5 * 3.0_f64.sqrt() * beta + zero;
    let c = -0.5 * alpha - 0.5 * 3.0_f64.sqrt() * beta + zero;
    [a, b, c]
}

/// `(alpha, beta, zero, theta) -> (d, q, zero)` — projects the stationary frame onto a frame
/// rotating at angle `theta` (radians). Zero-sequence passes through unchanged.
pub fn park(alpha: f64, beta: f64, zero: f64, theta: f64) -> [f64; 3] {
    let d = alpha * theta.cos() + beta * theta.sin();
    let q = -alpha * theta.sin() + beta * theta.cos();
    [d, q, zero]
}

/// `(d, q, zero, theta) -> (alpha, beta, zero)`. Exact inverse of [`park`] at the same `theta`.
pub fn park_inv(d: f64, q: f64, zero: f64, theta: f64) -> [f64; 3] {
    let alpha = d * theta.cos() - q * theta.sin();
    let beta = d * theta.sin() + q * theta.cos();
    [alpha, beta, zero]
}

/// `(a, b, c, theta) -> (d, q, zero)` directly, equivalent to `park(clarke(a, b, c), theta)`
/// (see `clarke_park_matches_clarke_then_park`) without the `alpha`/`beta` intermediate.
pub fn clarke_park(a: f64, b: f64, c: f64, theta: f64) -> [f64; 3] {
    let d = TWO_THIRDS
        * (a * theta.cos()
            + b * (theta - 2.0 * PI / 3.0).cos()
            + c * (theta + 2.0 * PI / 3.0).cos());
    let q = -TWO_THIRDS
        * (a * theta.sin()
            + b * (theta - 2.0 * PI / 3.0).sin()
            + c * (theta + 2.0 * PI / 3.0).sin());
    let zero = (a + b + c) / 3.0;
    [d, q, zero]
}

/// `(d, q, zero, theta) -> (a, b, c)` directly, equivalent to
/// `clarke_inv(park_inv(d, q, zero, theta), theta)`. Exact inverse of [`clarke_park`].
pub fn clarke_park_inv(d: f64, q: f64, zero: f64, theta: f64) -> [f64; 3] {
    let a = d * theta.cos() - q * theta.sin() + zero;
    let b = d * (theta - 2.0 * PI / 3.0).cos() - q * (theta - 2.0 * PI / 3.0).sin() + zero;
    let c = d * (theta + 2.0 * PI / 3.0).cos() - q * (theta + 2.0 * PI / 3.0).sin() + zero;
    [a, b, c]
}

/// `(alpha, beta) -> theta`, wrapped to `[0, 2*pi)` — the angle-tracking half of a
/// synchronous-reference-frame PLL: `atan2(beta, alpha)`, shifted by a full turn when negative
/// instead of the usual `(-pi, pi]` range, so `theta` is a monotonically-sensible input to
/// [`park`]/[`clarke_park`] across a full mechanical/electrical revolution.
pub fn angle_wrapped(alpha: f64, beta: f64) -> f64 {
    let theta = beta.atan2(alpha);
    if theta >= 0.0 {
        theta
    } else {
        theta + 2.0 * PI
    }
}

/// One of the six Clarke/Park coordinate transforms, grouped for dispatch by
/// `dae-runtime`'s block graph the same way [`crate::MathFn1`]/[`crate::MathFn2`] group the
/// single-output waveform-arithmetic functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinateTransform {
    Clarke,
    ClarkeInv,
    Park,
    ParkInv,
    ClarkePark,
    ClarkeParkInv,
}

impl CoordinateTransform {
    /// Number of inputs this transform expects, in the order [`Self::call`] requires them.
    pub fn input_count(self) -> usize {
        match self {
            CoordinateTransform::Clarke | CoordinateTransform::ClarkeInv => 3,
            CoordinateTransform::Park
            | CoordinateTransform::ParkInv
            | CoordinateTransform::ClarkePark
            | CoordinateTransform::ClarkeParkInv => 4,
        }
    }

    /// Conventional names for this transform's outputs, in the order [`Self::call`] returns
    /// them — a caller wiring up a `dae-runtime` block graph names each output block after one
    /// of these (e.g. `"pll_alpha"`, `"pll_beta"`, `"pll_zero"`).
    pub fn output_names(self) -> [&'static str; 3] {
        match self {
            CoordinateTransform::Clarke => ["alpha", "beta", "zero"],
            CoordinateTransform::ClarkeInv => ["a", "b", "c"],
            CoordinateTransform::Park | CoordinateTransform::ClarkePark => ["d", "q", "zero"],
            CoordinateTransform::ParkInv => ["alpha", "beta", "zero"],
            CoordinateTransform::ClarkeParkInv => ["a", "b", "c"],
        }
    }

    /// Evaluates this transform. `inputs.len()` must equal [`Self::input_count`] (the caller's
    /// responsibility — a device-file-level parser is the natural place to check once at load
    /// time, matching [`crate::waveform_arithmetic::table`]'s convention).
    pub fn call(self, inputs: &[f64]) -> [f64; 3] {
        match self {
            CoordinateTransform::Clarke => clarke(inputs[0], inputs[1], inputs[2]),
            CoordinateTransform::ClarkeInv => clarke_inv(inputs[0], inputs[1], inputs[2]),
            CoordinateTransform::Park => park(inputs[0], inputs[1], inputs[2], inputs[3]),
            CoordinateTransform::ParkInv => park_inv(inputs[0], inputs[1], inputs[2], inputs[3]),
            CoordinateTransform::ClarkePark => {
                clarke_park(inputs[0], inputs[1], inputs[2], inputs[3])
            }
            CoordinateTransform::ClarkeParkInv => {
                clarke_park_inv(inputs[0], inputs[1], inputs[2], inputs[3])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOL: f64 = 1e-9;

    fn assert_close3(got: [f64; 3], want: [f64; 3]) {
        for i in 0..3 {
            assert!(
                (got[i] - want[i]).abs() < TOL,
                "component {i}: got {}, want {}",
                got[i],
                want[i]
            );
        }
    }

    /// A balanced three-phase snapshot, hand-picked so every component is easy to check by
    /// hand: a=1, b=-0.5, c=-0.5 is `cos(0)`, `cos(-2pi/3)`, `cos(2pi/3)` scaled by amplitude 1
    /// — i.e. the `theta=0` instant of a unit-amplitude balanced waveform.
    const A: f64 = 1.0;
    const B: f64 = -0.5;
    const C: f64 = -0.5;

    #[test]
    fn clarke_matches_hand_derived_balanced_snapshot() {
        // alpha = (2/3)(1 - 0.5*(-0.5) - 0.5*(-0.5)) = (2/3)(1.5) = 1
        // beta = (b - c)/sqrt(3) = 0/sqrt(3) = 0
        // zero = (1 - 0.5 - 0.5)/3 = 0
        assert_close3(clarke(A, B, C), [1.0, 0.0, 0.0]);
    }

    #[test]
    fn clarke_inv_is_exact_inverse_of_clarke() {
        let [alpha, beta, zero] = clarke(A, B, C);
        assert_close3(clarke_inv(alpha, beta, zero), [A, B, C]);

        // A second, less-symmetric snapshot, to catch a round-trip bug the theta=0 case above
        // wouldn't (its beta happens to be zero).
        let (a, b, c) = (3.0, -1.0, 0.4);
        let [alpha, beta, zero] = clarke(a, b, c);
        assert_close3(clarke_inv(alpha, beta, zero), [a, b, c]);
    }

    #[test]
    fn park_inv_is_exact_inverse_of_park() {
        let (alpha, beta, zero, theta) = (0.7, -1.3, 0.2, 1.234);
        let [d, q, z] = park(alpha, beta, zero, theta);
        assert_close3(park_inv(d, q, z, theta), [alpha, beta, zero]);
    }

    /// The load-bearing property this whole module exists for: a balanced three-phase signal,
    /// tracked with `theta` locked to its own electrical angle, transforms to a *constant*
    /// `d`/`q` pair (amplitude on `d`, zero on `q` at this phase convention) — the reason a
    /// three-phase `Pid` on `q` works at all: it's regulating a DC quantity, not chasing a sine
    /// wave. Checked at several `theta` values along the rotation, not just `theta=0`.
    #[test]
    fn balanced_three_phase_at_its_own_angle_gives_constant_dq() {
        let amplitude = 2.0_f64;
        for &theta in &[0.0, 0.4, PI / 2.0, 2.1, 5.9] {
            let a = amplitude * theta.cos();
            let b = amplitude * (theta - 2.0 * PI / 3.0).cos();
            let c = amplitude * (theta + 2.0 * PI / 3.0).cos();
            let [d, q, zero] = clarke_park(a, b, c, theta);
            assert!((d - amplitude).abs() < TOL, "theta={theta}: d={d}");
            assert!(q.abs() < TOL, "theta={theta}: q={q}");
            assert!(zero.abs() < TOL, "theta={theta}: zero={zero}");
        }
    }

    /// `clarke_park`'s fused abc->dq0 must agree with composing the two separate transforms —
    /// the upstream library this was adapted from documents having once shipped `clarke_park`
    /// with `d`/`q` swapped relative to the two-step composition, so this equivalence is the
    /// direct regression test for that class of bug, not just a nice-to-have cross-check.
    #[test]
    fn clarke_park_matches_clarke_then_park() {
        let (a, b, c, theta) = (1.1, -0.6, -0.3, 0.77);
        let [alpha, beta, zero] = clarke(a, b, c);
        let composed = park(alpha, beta, zero, theta);
        let fused = clarke_park(a, b, c, theta);
        assert_close3(fused, composed);
    }

    #[test]
    fn clarke_park_inv_matches_park_inv_then_clarke_inv() {
        let (d, q, zero, theta) = (0.9, -0.2, 0.1, 2.3);
        let [alpha, beta, z] = park_inv(d, q, zero, theta);
        let composed = clarke_inv(alpha, beta, z);
        let fused = clarke_park_inv(d, q, zero, theta);
        assert_close3(fused, composed);
    }

    #[test]
    fn clarke_park_inv_is_exact_inverse_of_clarke_park() {
        let (a, b, c, theta) = (2.0, -1.0, -1.0, 0.5);
        let [d, q, zero] = clarke_park(a, b, c, theta);
        assert_close3(clarke_park_inv(d, q, zero, theta), [a, b, c]);
    }

    #[test]
    fn angle_wrapped_matches_hand_derived_quadrants_and_wraps_negative_into_0_2pi() {
        // First quadrant: alpha=1, beta=1 -> 45 degrees = pi/4.
        assert!((angle_wrapped(1.0, 1.0) - PI / 4.0).abs() < TOL);
        // Straight up: alpha=0, beta=1 -> pi/2.
        assert!((angle_wrapped(0.0, 1.0) - PI / 2.0).abs() < TOL);
        // Third quadrant would be a negative atan2 result (-3pi/4); wrapped, it's 5pi/4.
        assert!((angle_wrapped(-1.0, -1.0) - 5.0 * PI / 4.0).abs() < TOL);
        // Straight down: alpha=0, beta=-1 -> raw atan2 is -pi/2, wrapped is 3pi/2.
        assert!((angle_wrapped(0.0, -1.0) - 3.0 * PI / 2.0).abs() < TOL);
    }

    #[test]
    fn angle_wrapped_tracks_a_full_rotation_consistent_with_clarke() {
        // theta swept across a full turn: clarke() of a balanced signal at that theta should
        // recover the same theta via angle_wrapped (alpha/beta live on the unit circle scaled
        // by amplitude, so their angle is theta itself).
        let amplitude = 1.0_f64;
        for &theta in &[0.0, 0.1, PI / 2.0, PI, 3.0 * PI / 2.0 + 0.2, 2.0 * PI - 0.1] {
            let a = amplitude * theta.cos();
            let b = amplitude * (theta - 2.0 * PI / 3.0).cos();
            let c = amplitude * (theta + 2.0 * PI / 3.0).cos();
            let [alpha, beta, _zero] = clarke(a, b, c);
            let recovered = angle_wrapped(alpha, beta);
            assert!(
                (recovered - theta).abs() < 1e-6,
                "theta={theta}: recovered={recovered}"
            );
        }
    }

    #[test]
    fn coordinate_transform_dispatch_matches_direct_calls() {
        let (a, b, c, theta) = (1.0, -0.5, -0.5, 0.3);
        assert_eq!(
            CoordinateTransform::Clarke.call(&[a, b, c]),
            clarke(a, b, c)
        );
        assert_eq!(CoordinateTransform::Clarke.input_count(), 3);
        assert_eq!(
            CoordinateTransform::Clarke.output_names(),
            ["alpha", "beta", "zero"]
        );

        assert_eq!(
            CoordinateTransform::ClarkePark.call(&[a, b, c, theta]),
            clarke_park(a, b, c, theta)
        );
        assert_eq!(CoordinateTransform::ClarkePark.input_count(), 4);
        assert_eq!(
            CoordinateTransform::ClarkePark.output_names(),
            ["d", "q", "zero"]
        );
    }
}
