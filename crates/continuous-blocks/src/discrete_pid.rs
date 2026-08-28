//! A discrete-time PID (P/I/PI/PD/PID), realized directly from its own block diagram (three
//! parallel branches, summed) rather than by combining into one z-domain transfer function and
//! converting back to state-space — see `book/dev-guide/src/discrete-time-blocks.md`'s own
//! Category 3 for why, and for the source-manual correction (a docling reading-order swap, not
//! assumed) the integral/derivative formulas below depend on.
//!
//! The derivative action's own internal low-pass filter is **always** realized with the
//! Forward Euler integrator formula, regardless of what [`DiscreteIntegrationMethod`] the
//! integral action uses — not an arbitrary choice: Forward Euler is the only one of the three
//! methods with no direct feedthrough, and this filter's own topology (its integrator's output
//! feeds back into the very sum that computes the integrator's next input) would be an
//! unresolvable same-step algebraic loop under Backward Euler or Trapezoidal, both of which
//! *do* have feedthrough.

/// How the integral action's own discrete integrator advances — see each variant's own
/// recursion in [`DiscretePid::step`]. Verified directly against the source manual's own PDF
/// (not the initial, swapped automated conversion — see this module's own doc comment).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiscreteIntegrationMethod {
    /// `y[k] = y[k-1] + T*e[k-1]` — no direct feedthrough (`y[k]` never references `e[k]`).
    ForwardEuler,
    /// `y[k] = y[k-1] + T*e[k]` — direct feedthrough.
    BackwardEuler,
    /// `y[k] = y[k-1] + (T/2)*(e[k-1]+e[k])` — direct feedthrough.
    Trapezoidal,
}

impl DiscreteIntegrationMethod {
    pub fn from_name(name: &str) -> Option<Self> {
        use DiscreteIntegrationMethod::*;
        Some(match name {
            "forward" => ForwardEuler,
            "backward" => BackwardEuler,
            "trapezoidal" => Trapezoidal,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        use DiscreteIntegrationMethod::*;
        match self {
            ForwardEuler => "forward",
            BackwardEuler => "backward",
            Trapezoidal => "trapezoidal",
        }
    }
}

/// A discrete PID's own configuration — `kp`/`ki`/`kd` the usual gains, `n` the derivative
/// filter coefficient (this codebase's own existing name for the concept the source manual
/// calls `Kf` — same idea, kept consistent with the continuous [`crate::Pid`]'s own field name
/// rather than importing a second spelling for it), `period` the sample period `T` baked in at
/// construction (from the declaring block's own `SampleTimeSpec::Periodic`, not a separate
/// netlist field — the discretized recursion's own coefficients need the *same* period the
/// block is scheduled at, so there is exactly one place to declare it).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiscretePid {
    pub kp: f64,
    pub ki: f64,
    pub kd: f64,
    pub n: f64,
    pub period: f64,
    pub method: DiscreteIntegrationMethod,
}

/// [`DiscretePid`]'s own persisted state — two small scalars per branch that needs memory (the
/// proportional action is stateless). `int_prev_e` is only ever read/written when `method` is
/// `ForwardEuler`/`Trapezoidal` (both need a remembered previous error); `BackwardEuler` never
/// touches it, but it's still always present in this struct rather than modeled as an
/// `Option`/per-method enum, since a fixed-shape state is simpler for `dae-runtime` to hold
/// regardless of which method a given instance happens to use.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct DiscretePidState {
    /// The integral action's own accumulated value (`Ki`-scaled contribution to `u` is
    /// `ki * int_state` after each step's own update — see [`DiscretePid::step`]).
    pub int_state: f64,
    /// The integral action's own remembered previous error — only meaningful for
    /// `ForwardEuler`/`Trapezoidal`.
    pub int_prev_e: f64,
    /// The derivative filter's own accumulated value.
    pub filt_state: f64,
    /// The derivative filter's own remembered previous input (`v[k-1]`) — always meaningful,
    /// since the filter is always Forward Euler.
    pub filt_prev_v: f64,
}

impl DiscretePid {
    /// One step: given this step's own error `e`, advances `state` in place and returns the
    /// **unsaturated** controller output `u[k] = Kp*e[k] + Ki*int_output[k] + Kd*v[k]` — the
    /// caller (`dae-runtime`) applies anti-windup clamping on top of this, the same
    /// tentative-step/reject logic the continuous `Pid`'s own arm already uses (this method
    /// itself has no notion of saturation limits at all).
    pub fn step(&self, state: &mut DiscretePidState, e: f64) -> f64 {
        // Integral action: a plain discrete integrator on the raw error, using whichever method
        // this instance was configured with -- the corrected formulas from
        // `discrete-time-blocks.md`'s own Category 1 survey.
        let int_output = match self.method {
            DiscreteIntegrationMethod::ForwardEuler => {
                let y = state.int_state + self.period * state.int_prev_e;
                state.int_state = y;
                state.int_prev_e = e;
                y
            }
            DiscreteIntegrationMethod::BackwardEuler => {
                let y = state.int_state + self.period * e;
                state.int_state = y;
                y
            }
            DiscreteIntegrationMethod::Trapezoidal => {
                let y = state.int_state + (self.period / 2.0) * (state.int_prev_e + e);
                state.int_state = y;
                state.int_prev_e = e;
                y
            }
        };

        // Derivative action: always Forward Euler (see this module's own doc comment for why).
        // `filt_state` is read *before* being updated -- no feedthrough, so `v` can be computed
        // from this step's own `e` with no algebraic loop.
        let v = self.n * (self.kd * e - state.filt_state);
        let filt_next = state.filt_state + self.period * state.filt_prev_v;
        state.filt_prev_v = v;
        state.filt_state = filt_next;

        self.kp * e + self.ki * int_output + v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pure_p_is_stateless_and_matches_kp_times_error_every_step() {
        let pid = DiscretePid {
            kp: 3.0,
            ki: 0.0,
            kd: 0.0,
            n: 1.0,
            period: 0.01,
            method: DiscreteIntegrationMethod::ForwardEuler,
        };
        let mut state = DiscretePidState::default();
        assert_eq!(pid.step(&mut state, 2.0), 6.0);
        assert_eq!(pid.step(&mut state, -1.0), -3.0); // no memory of the previous call
    }

    #[test]
    fn forward_euler_integral_matches_the_hand_derived_no_feedthrough_recursion() {
        // y[k] = y[k-1] + T*e[k-1] -- e held at 2.0 throughout, T=0.1, Ki=5.
        let pid = DiscretePid {
            kp: 0.0,
            ki: 5.0,
            kd: 0.0,
            n: 1.0,
            period: 0.1,
            method: DiscreteIntegrationMethod::ForwardEuler,
        };
        let mut state = DiscretePidState::default();
        // Step 1: int_state=0 + 0.1*int_prev_e(=0, default) = 0 -> u = 5*0 = 0.
        assert_eq!(pid.step(&mut state, 2.0), 0.0);
        // Step 2: int_state=0 + 0.1*2.0 (prev e) = 0.2 -> u = 5*0.2 = 1.0.
        assert_eq!(pid.step(&mut state, 2.0), 1.0);
        // Step 3: int_state=0.2 + 0.1*2.0 = 0.4 -> u = 5*0.4 = 2.0.
        assert_eq!(pid.step(&mut state, 2.0), 2.0);
    }

    #[test]
    fn backward_euler_integral_has_feedthrough_unlike_forward_euler() {
        // y[k] = y[k-1] + T*e[k] -- same e/T/Ki as above, but should react on the *first* call
        // (feedthrough), unlike Forward Euler's zero first output above.
        let pid = DiscretePid {
            kp: 0.0,
            ki: 5.0,
            kd: 0.0,
            n: 1.0,
            period: 0.1,
            method: DiscreteIntegrationMethod::BackwardEuler,
        };
        let mut state = DiscretePidState::default();
        // Step 1: int_state=0 + 0.1*2.0 = 0.2 -> u = 5*0.2 = 1.0 (nonzero on the first call).
        assert_eq!(pid.step(&mut state, 2.0), 1.0);
        assert_eq!(pid.step(&mut state, 2.0), 2.0);
    }

    #[test]
    fn trapezoidal_integral_averages_current_and_previous_error() {
        let pid = DiscretePid {
            kp: 0.0,
            ki: 1.0,
            kd: 0.0,
            n: 1.0,
            period: 1.0,
            method: DiscreteIntegrationMethod::Trapezoidal,
        };
        let mut state = DiscretePidState::default();
        // Step 1: y = 0 + (1/2)*(0 (prev, default) + 2.0) = 1.0.
        assert_eq!(pid.step(&mut state, 2.0), 1.0);
        // Step 2: y = 1.0 + (1/2)*(2.0 + 4.0) = 1.0 + 3.0 = 4.0.
        assert_eq!(pid.step(&mut state, 4.0), 4.0);
    }

    #[test]
    fn pure_d_settles_to_zero_for_a_constant_error_a_real_derivative_property() {
        // A real derivative action's output must decay to zero when its own input stops
        // changing -- checked here numerically (many steps at constant e), not just that the
        // recursion runs.
        let pid = DiscretePid {
            kp: 0.0,
            ki: 0.0,
            kd: 2.0,
            n: 50.0, // a fast filter pole so this settles within a modest number of steps
            period: 0.001,
            method: DiscreteIntegrationMethod::ForwardEuler,
        };
        let mut state = DiscretePidState::default();
        let mut last = f64::MAX;
        for _ in 0..2000 {
            last = pid.step(&mut state, 3.0); // constant error
        }
        assert!(
            last.abs() < 1e-6,
            "expected settled derivative ~0, got {last}"
        );
    }

    #[test]
    fn from_name_matches_conventional_names_and_rejects_unknown() {
        assert_eq!(
            DiscreteIntegrationMethod::from_name("backward"),
            Some(DiscreteIntegrationMethod::BackwardEuler)
        );
        assert_eq!(DiscreteIntegrationMethod::from_name("nope"), None);
    }
}
