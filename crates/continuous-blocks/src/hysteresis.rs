/// A Schmitt-trigger comparator: stays HIGH until the input drops below `low`, stays LOW
/// until the input rises above `high` — the standard bang-bang/hysteresis-band block
/// (block-diagram simulation tools typically offer an equivalent "Relay"/hysteresis block)
/// used for current-mode control when there's no fixed switching frequency to modulate a duty
/// command onto, unlike [`crate::Pid`]
/// feeding a PWM carrier. Deliberately not a [`crate::StateSpace`]: the on/off memory is a
/// genuine discrete latch, not a linear dynamic, the same reason [`crate::Vco`]'s wraparound
/// is evaluated directly rather than folded into one.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hysteresis {
    pub high: f64,
    pub low: f64,
}

impl Hysteresis {
    pub fn new(high: f64, low: f64) -> Self {
        assert!(low <= high, "low must not exceed high");
        Hysteresis { high, low }
    }

    /// Advances the latch by one step. `prev_state` is the previous step's output (`true` =
    /// HIGH), `x` this step's input. Returns the new state.
    pub fn step(&self, prev_state: bool, x: f64) -> bool {
        if prev_state {
            x >= self.low
        } else {
            x > self.high
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rises_above_high_to_turn_on() {
        let h = Hysteresis::new(1.0, -1.0);
        assert!(!h.step(false, 0.5));
        assert!(h.step(false, 1.5));
    }

    #[test]
    fn stays_on_inside_the_band() {
        let h = Hysteresis::new(1.0, -1.0);
        assert!(h.step(true, 0.0));
        assert!(h.step(true, -0.5));
    }

    #[test]
    fn falls_below_low_to_turn_off() {
        let h = Hysteresis::new(1.0, -1.0);
        assert!(!h.step(true, -1.5));
    }

    #[test]
    fn stays_off_inside_the_band() {
        let h = Hysteresis::new(1.0, -1.0);
        assert!(!h.step(false, 0.0));
        assert!(!h.step(false, 0.9));
    }

    #[test]
    fn boundary_values_are_inclusive_of_staying_in_the_current_state() {
        let h = Hysteresis::new(1.0, -1.0);
        // exactly at low, while ON, should stay ON (>= low)
        assert!(h.step(true, -1.0));
        // exactly at high, while OFF, should stay OFF (> high, not >=)
        assert!(!h.step(false, 1.0));
    }
}
