/// A voltage-controlled oscillator: given a frequency command (Hz, clamped to `[f_min,
/// f_max]`), outputs a repeating ramp in `[0, 1)` — the same standard block a reference tool/a reference tool
/// libraries provide (their own "VCO"/"Repeating Sequence" blocks), used here to build a
/// frequency-modulated PWM carrier. LLC-family resonant converters regulate by switching
/// *frequency*, not duty, unlike buck/boost, which is why this block exists alongside `Pid`
/// rather than folding frequency modulation into `Pid` or into `dae-runtime` directly — a
/// closed loop wires `Sum -> Pid -> Gain -> Vco` as separate, independently reusable blocks,
/// the same way a real block-diagram tool would, rather than one function that hardcodes the
/// whole topology.
///
/// Deliberately not a [`crate::StateSpace`]: the wraparound is a genuine discontinuity a
/// linear system can't express, the same reason [`crate::math_ops::saturation`] is evaluated
/// directly rather than folded into one. Composing `freq = f_nom + k*control` is the caller's
/// job (chain a [`crate::math_ops::gain`] block, or a `Sum` with a constant, ahead of this
/// one) — this block's only responsibility is the oscillator itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vco {
    pub f_min: f64,
    pub f_max: f64,
}

impl Vco {
    pub fn new(f_min: f64, f_max: f64) -> Self {
        assert!(f_min <= f_max, "f_min must not exceed f_max");
        Vco { f_min, f_max }
    }

    /// Advances the oscillator by one step. `phase` is the previous step's output (in `[0,
    /// 1)`, the block's own state), `freq_command` this step's input (Hz, clamped internally
    /// to `[f_min, f_max]`), `dt` the step size. Returns the new phase, also in `[0, 1)`.
    pub fn step(&self, phase: f64, freq_command: f64, dt: f64) -> f64 {
        let freq = freq_command.clamp(self.f_min, self.f_max);
        (phase + freq * dt).rem_euclid(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_at_the_commanded_frequency_before_wrapping() {
        let vco = Vco::new(0.0, 200_000.0);
        let phase = vco.step(0.0, 100_000.0, 1e-8);
        assert!((phase - 1e-3).abs() < 1e-12);
    }

    #[test]
    fn wraps_around_at_one() {
        let vco = Vco::new(0.0, 200_000.0);
        // freq=100_000, dt=1e-5 -> raw phase advance = 1.0 exactly -> wraps to 0.0
        let phase = vco.step(0.0, 100_000.0, 1e-5);
        assert!(phase.abs() < 1e-9);
        // starting from 0.5, the same step should land back at 0.5
        let phase2 = vco.step(0.5, 100_000.0, 1e-5);
        assert!((phase2 - 0.5).abs() < 1e-9);
    }

    #[test]
    fn clamps_frequency_command_to_range() {
        let vco = Vco::new(50_000.0, 100_000.0);
        // command far above f_max should behave as if commanded exactly at f_max
        let phase_over = vco.step(0.0, 1_000_000.0, 1e-8);
        let phase_at_max = vco.step(0.0, 100_000.0, 1e-8);
        assert_eq!(phase_over, phase_at_max);
        // command far below f_min (even negative) should clamp to f_min
        let phase_under = vco.step(0.0, -1_000_000.0, 1e-8);
        let phase_at_min = vco.step(0.0, 50_000.0, 1e-8);
        assert_eq!(phase_under, phase_at_min);
    }
}
