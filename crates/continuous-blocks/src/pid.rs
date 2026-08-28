use crate::state_space::StateSpace;
use crate::transfer_function::TransferFunction;

/// <!-- doc-ref: pid -->
/// Parallel PID with a filtered derivative term, using the standard `P, I, D, N` (filter
/// coefficient) parameterization common to block-diagram continuous-time controller blocks:
///
/// $$C(s) = K_p + \frac{K_i}{s} + \frac{K_d N s}{s + N}$$
///
/// The pure-derivative term `Kd*s` alone is not realizable as a proper (causal) transfer
/// function, hence the filter pole at `-N` — a standard, universally-used approximation (a
/// larger `N` tracks the ideal derivative more closely but stiffens the resulting ODE).
///
/// This struct is the compiled gain set only — see `general_mna::block_graph::BlockKind::Pid`
/// for the netlist-facing component reference (parameters, errors, netlist form, example) and
/// the anti-windup behavior, which lives at the block-graph evaluation layer, not here.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pid {
    /// Proportional gain.
    pub kp: f64,
    /// Integral gain.
    pub ki: f64,
    /// Derivative gain.
    pub kd: f64,
    /// Derivative filter coefficient; the filter pole sits at `-n`. Must be `> 0` — see
    /// [`Pid::new`].
    pub n: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PidError {
    /// `n <= 0` puts the derivative filter's pole at `-n`, i.e. at the origin or in the
    /// right half-plane — a non-causal or outright unstable filter, not the standard,
    /// universally-used approximation this parameterization exists to give.
    NonPositiveFilterCoefficient,
}

impl Pid {
    pub fn new(kp: f64, ki: f64, kd: f64, n: f64) -> Result<Self, PidError> {
        if n <= 0.0 {
            return Err(PidError::NonPositiveFilterCoefficient);
        }
        Ok(Pid { kp, ki, kd, n })
    }

    /// Puts `C(s)` over the common denominator `s*(s+N)`:
    ///
    /// ```text
    /// C(s) = [(Kp+Kd*N)*s^2 + (Kp*N+Ki)*s + Ki*N] / [s^2 + N*s]
    /// ```
    ///
    /// Derived by hand before implementation (see `docs/architecture.md`'s Milestone 5 note):
    /// with `Kd=0` this numerator factors exactly as `(s+N)(Kp*s+Ki)`, so the filter pole/zero
    /// cancel and the closed-loop step response of the resulting state-space realization
    /// reduces to the ideal PI response `y(t) = Kp + Ki*t` for *every* `t`, not just
    /// asymptotically — `tests/step_responses.rs` checks exactly this.
    pub fn to_transfer_function(&self) -> TransferFunction {
        let num = vec![
            self.kp + self.kd * self.n,
            self.kp * self.n + self.ki,
            self.ki * self.n,
        ];
        let den = vec![1.0, self.n, 0.0];
        TransferFunction::new(num, den).expect("PID transfer function is always proper")
    }

    pub fn to_state_space(&self) -> StateSpace {
        self.to_transfer_function().to_state_space()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kd_zero_cancels_to_pure_pi_numerator() {
        // With Kd=0, numerator should equal (s+N)(Kp*s+Ki) expanded, which is what
        // to_transfer_function already computes directly -- this test instead cross-checks it
        // against the expanded-by-hand form for a concrete (Kp, Ki, N), as an independent
        // arithmetic check of the formula itself (not just that the code returns its own
        // formula back).
        let pid = Pid::new(2.0, 3.0, 0.0, 10.0).unwrap();
        let tf = pid.to_transfer_function();
        // (s+10)(2s+3) = 2s^2 + 3s + 20s + 30 = 2s^2 + 23s + 30
        assert_eq!(tf.num, vec![2.0, 23.0, 30.0]);
        assert_eq!(tf.den, vec![1.0, 10.0, 0.0]);
    }

    #[test]
    fn non_positive_filter_coefficient_is_rejected_not_silently_unstable() {
        assert_eq!(
            Pid::new(2.0, 3.0, 0.5, 0.0),
            Err(PidError::NonPositiveFilterCoefficient)
        );
        assert_eq!(
            Pid::new(2.0, 3.0, 0.5, -10.0),
            Err(PidError::NonPositiveFilterCoefficient)
        );
    }
}
