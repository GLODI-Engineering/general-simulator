//! `Math Operations`-style blocks that don't need state: gain, sum, product, saturation.
//! These are pure functions of their inputs, not descriptor-DAE fragments — a `dae-runtime`
//! integration (a later milestone) would evaluate them inline while assembling the global
//! system rather than adding unknowns/rows for them, the same way a resistor's Ohm's-law
//! relationship needs no extra unknown.

pub fn gain(k: f64, u: f64) -> f64 {
    k * u
}

pub fn sum(inputs: &[f64], signs: &[f64]) -> f64 {
    assert_eq!(inputs.len(), signs.len(), "one sign per input");
    inputs.iter().zip(signs).map(|(u, s)| u * s).sum()
}

pub fn product(inputs: &[f64]) -> f64 {
    inputs.iter().product()
}

/// Compares a periodic ramp (as produced by [`crate::Vco`], in `[0, 1)`) against a duty
/// threshold, with an optional phase offset — the standard way a PWM carrier or oscillator
/// output becomes a logic/gate signal (block-diagram simulation tools typically offer an
/// equivalent relational-operator-on-a-ramp block). Kept separate from `Vco` itself so one
/// shared oscillator can
/// drive several independently-phased/duty-shifted gates (e.g. a half-bridge's two
/// complementary switches) without needing a second oscillator instance.
pub fn pwm_from_ramp(ramp: f64, phase_offset: f64, duty: f64) -> bool {
    (ramp + phase_offset).rem_euclid(1.0) < duty
}

/// Clamps `u` to `[-limit, limit]`. Piecewise, like a PWL device, but evaluated directly here
/// rather than through the LCP machinery: a saturation block's active segment depends only on
/// its own input, never on other unknowns it doesn't already know, so there is no
/// simultaneous-resolution problem the way there is for a diode's terminal voltage (which
/// depends on the very unknowns the LCP is solving for).
pub fn saturation(u: f64, limit: f64) -> f64 {
    assert!(limit >= 0.0, "limit must be nonnegative");
    u.clamp(-limit, limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_scales() {
        assert_eq!(gain(3.0, 4.0), 12.0);
    }

    #[test]
    fn sum_applies_signs() {
        assert_eq!(sum(&[5.0, 2.0, 1.0], &[1.0, -1.0, 1.0]), 4.0);
    }

    #[test]
    fn product_multiplies_all_inputs() {
        assert_eq!(product(&[2.0, 3.0, 4.0]), 24.0);
    }

    #[test]
    fn saturation_clamps_both_sides() {
        assert_eq!(saturation(5.0, 2.0), 2.0);
        assert_eq!(saturation(-5.0, 2.0), -2.0);
        assert_eq!(saturation(1.0, 2.0), 1.0);
    }

    #[test]
    fn pwm_from_ramp_compares_against_duty_with_phase_offset() {
        assert!(pwm_from_ramp(0.1, 0.0, 0.48));
        assert!(!pwm_from_ramp(0.5, 0.0, 0.48));
        // phase_offset=0.5 shifts the comparison window by half a period
        assert!(pwm_from_ramp(0.6, 0.5, 0.48));
        assert!(!pwm_from_ramp(0.1, 0.5, 0.48));
    }
}
