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

/// Compares a carrier phase `theta` (`[0, 1)`, e.g. a periodic ramp's own `(ramp +
/// phase_offset).rem_euclid(1.0)`, as produced by [`crate::Vco`], or
/// [`crate::closed_loop::sawtooth_carrier`]) against `duty`,
/// producing an **active-high complementary pair** `(main, complement)` with independently
/// configurable dead time on each edge — the shared primitive every gate-driving modulator in
/// this crate uses, so "how dead time is inserted" only has one implementation to get right.
///
/// Without dead time, `main` is on for `theta` in `[0, duty)` and `complement` is on for
/// `theta` in `[duty, 1)` — an exact partition, no gap, no overlap (this is what `red_frac =
/// fed_frac = 0.0` recovers exactly). Real dead time delays only the two **rising** (turn-on)
/// edges, never a falling (turn-off) edge — the standard technique (the same one real hardware
/// timer dead-time generators use) that guarantees both outputs are provably low during the
/// gap, since whichever switch was conducting always turns off exactly on schedule and the
/// *other* one is held off a little longer before it's allowed to turn on:
/// - `main`'s own rising edge (at `theta=0`) is delayed by `red_frac` (Rising-Edge Delay, as a
///   fraction of one carrier period) — `main` is on for `theta` in `[red_frac, duty)`.
/// - `complement`'s own rising edge (at `theta=duty`) is delayed by `fed_frac` (Falling-Edge
///   Delay — named for *which duty transition* it guards, `main`'s own falling edge, not
///   because it delays a falling edge itself; it never does) — `complement` is on for `theta`
///   in `[duty + fed_frac, 1)`.
///
/// Both dead gaps this leaves — `[0, red_frac)` and `[duty, duty+fed_frac)` — have `main` and
/// `complement` simultaneously off. Callers are responsible for keeping `red_frac`/`fed_frac`
/// small relative to `duty`/`1-duty` respectively (this function doesn't validate that — a
/// `red_frac` exceeding `duty`, for instance, just makes `main` never turn on that cycle rather
/// than panicking, the same "degenerate but not a crash" policy [`saturation`]'s own limit
/// clamp uses).
pub fn complementary_pwm_with_deadtime(
    theta: f64,
    duty: f64,
    red_frac: f64,
    fed_frac: f64,
) -> (bool, bool) {
    let duty = duty.clamp(0.0, 1.0);
    let red_frac = red_frac.max(0.0);
    let fed_frac = fed_frac.max(0.0);
    let main = theta >= red_frac && theta < duty;
    let complement = theta >= duty + fed_frac;
    (main, complement)
}

/// <!-- doc-ref: saturation -->
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
    fn complementary_pwm_with_zero_deadtime_is_an_exact_partition() {
        // duty=0.3, no dead time: main on [0,0.3), complement on [0.3,1) -- exactly one of the
        // two is on at every theta, never both, never neither.
        for theta in [0.0, 0.1, 0.29, 0.3, 0.5, 0.99] {
            let (main, comp) = complementary_pwm_with_deadtime(theta, 0.3, 0.0, 0.0);
            assert_ne!(
                main, comp,
                "theta={theta}: exactly one of main/complement must be on"
            );
        }
    }

    #[test]
    fn complementary_pwm_with_deadtime_leaves_both_off_during_each_gap() {
        // duty=0.3, red_frac=0.05, fed_frac=0.02: main on [0.05,0.3), complement on [0.32,1).
        let cases = [
            (0.0, false, false),  // gap 1: main not yet risen, complement already fell
            (0.03, false, false), // still gap 1
            (0.05, true, false),  // main's delayed rising edge
            (0.2, true, false),   // mid main-on
            (0.3, false, false),  // main's (undelayed) falling edge -> gap 2 starts
            (0.31, false, false), // still gap 2
            (0.32, false, true),  // complement's delayed rising edge
            (0.9, false, true),   // mid complement-on
        ];
        for (theta, exp_main, exp_comp) in cases {
            let (main, comp) = complementary_pwm_with_deadtime(theta, 0.3, 0.05, 0.02);
            assert_eq!(
                (main, comp),
                (exp_main, exp_comp),
                "theta={theta}: expected (main={exp_main}, complement={exp_comp})"
            );
        }
    }

    #[test]
    fn complementary_pwm_never_both_on_regardless_of_deadtime() {
        // Sweep a fine grid: whatever red_frac/fed_frac, main and complement must never both
        // be true simultaneously (the whole point of dead time is preventing shoot-through).
        for i in 0..1000 {
            let theta = i as f64 / 1000.0;
            let (main, comp) = complementary_pwm_with_deadtime(theta, 0.4, 0.05, 0.1);
            assert!(!(main && comp), "theta={theta}: both on simultaneously");
        }
    }
}
