use crate::state_space::StateSpace;
use crate::transfer_function::TransferFunction;

/// `1/s`: `dx/dt = u`, `y = x`. The initial condition is a property of the *simulation*
/// (the state vector's starting value), not of this structural description.
pub fn integrator() -> StateSpace {
    StateSpace {
        a: vec![vec![0.0]],
        b: vec![vec![1.0]],
        c: vec![vec![1.0]],
        d: vec![vec![0.0]],
        e: None,
    }
}

/// An approximate ("dirty"/filtered) derivative, `s / (tau*s + 1)`. A pure differentiator
/// (`Kd*s` alone) is not a proper transfer function — it isn't realizable as a causal ODE
/// state-space system — so every real implementation (this one, standard block-diagram PID
/// blocks, and analog PID compensators alike) uses a first-order low-pass filter pole to make
/// it proper.
/// Smaller `tau` tracks the ideal derivative more closely but stiffens the resulting ODE.
pub fn derivative_filtered(tau: f64) -> StateSpace {
    assert!(tau > 0.0, "filter time constant must be positive");
    TransferFunction::new(vec![1.0, 0.0], vec![tau, 1.0])
        .expect("s/(tau*s+1) is always proper")
        .to_state_space()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integrator_matches_textbook_single_state_form() {
        let ss = integrator();
        assert_eq!(ss.a, vec![vec![0.0]]);
        assert_eq!(ss.b, vec![vec![1.0]]);
        assert_eq!(ss.c, vec![vec![1.0]]);
        assert_eq!(ss.d, vec![vec![0.0]]);
    }

    /// s/(tau*s+1) = (1/tau) - (1/tau)/(s+1/tau), i.e. leading-coefficient division gives
    /// d = 1/tau directly, and the remainder pole sits at -1/tau -- hand-checked here for
    /// tau=0.1 (pole at -10, d=10).
    #[test]
    fn filtered_derivative_matches_hand_derived_pole_and_feedthrough() {
        let ss = derivative_filtered(0.1);
        assert!((ss.d[0][0] - 10.0).abs() < 1e-9);
        assert!((ss.a[0][0] - (-10.0)).abs() < 1e-9);
    }
}
