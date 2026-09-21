use crate::state_space::StateSpace;

/// A single-input single-output transfer function `N(s) / D(s)`, coefficients highest-degree
/// first (e.g. `s + 1` is `[1.0, 1.0]`). `D` must be nonzero-leading (degree = `den.len() - 1`)
/// and `deg(N) <= deg(D)` — a proper transfer function; this is what every realizable
/// continuous-time physical system is.
#[derive(Debug, Clone, PartialEq)]
pub struct TransferFunction {
    pub num: Vec<f64>,
    pub den: Vec<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferFunctionError {
    EmptyDenominator,
    ZeroLeadingDenominatorCoefficient,
    ImproperTransferFunction,
}

/// Why [`TransferFunction::settled_state`]/[`TransferFunction::settled_state_discrete`] could
/// not produce a state for the requested output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettledStateError {
    /// `deg(D) = 0` — a pure gain has no state to initialize at all.
    NoStates,
    /// The numerator vanishes at DC ($N(0) = 0$ in continuous time, $N(1) = 0$ in discrete
    /// time), so the settled output is identically zero and no state holds a nonzero one.
    ZeroDcNumerator,
}

impl TransferFunction {
    pub fn new(num: Vec<f64>, den: Vec<f64>) -> Result<Self, TransferFunctionError> {
        if den.is_empty() {
            return Err(TransferFunctionError::EmptyDenominator);
        }
        if den[0] == 0.0 {
            return Err(TransferFunctionError::ZeroLeadingDenominatorCoefficient);
        }
        if num.len() > den.len() {
            return Err(TransferFunctionError::ImproperTransferFunction);
        }
        Ok(TransferFunction { num, den })
    }

    /// Realizes this transfer function in controllable canonical form.
    ///
    /// `D(s) = s^n + a_{n-1} s^{n-1} + ... + a_0` (monic, after normalizing by the original
    /// leading coefficient). If `deg(N) == n`, one step of polynomial long division extracts
    /// the direct feedthrough `d = N[0]` (after normalization) and leaves a strictly proper
    /// remainder `R(s) = b_{n-1} s^{n-1} + ... + b_0`; otherwise `d = 0` and `R = N`.
    ///
    /// ```text
    /// A = [ 0      1      0     ...  0    ]      B = [0]
    ///     [ 0      0      1     ...  0    ]          [0]
    ///     [ ...                             ]         [...]
    ///     [ 0      0      0     ...  1    ]          [0]
    ///     [-a_0   -a_1   -a_2   ... -a_{n-1}]         [1]
    ///
    /// C = [b_0, b_1, ..., b_{n-1}]      D = [d]
    /// ```
    ///
    /// Verified against a first-order case by hand (`docs`/tests): for `1/(s+1)`, this gives
    /// exactly `A=[[-1]], B=[[1]], C=[[1]], D=[[0]]`, the textbook single-state realization.
    pub fn to_state_space(&self) -> StateSpace {
        let leading = self.den[0];
        let n = self.den.len() - 1;

        // Monic denominator coefficients a_0..a_{n-1} (a_i is the coefficient of s^i).
        let mut a_coeffs = vec![0.0; n];
        for (power, coeff) in self.den[1..].iter().rev().enumerate() {
            a_coeffs[power] = coeff / leading;
        }

        // Numerator, normalized by the same leading coefficient, padded to length n+1
        // (highest-degree first) so direct feedthrough extraction is a uniform subtraction.
        let mut num_norm = vec![0.0; n + 1 - self.num.len()];
        num_norm.extend(self.num.iter().map(|c| c / leading));

        let d_feedthrough = num_norm[0];
        // Remainder = N(s) - d * D_monic(s), both already highest-degree first, length n+1;
        // dropping the (now-zero) leading term leaves the n remaining coefficients.
        let mut den_monic = vec![1.0];
        den_monic.extend(a_coeffs.iter().rev());
        let remainder: Vec<f64> = num_norm
            .iter()
            .zip(den_monic.iter())
            .map(|(ni, di)| ni - d_feedthrough * di)
            .collect();
        // b_i is the coefficient of s^i; remainder[1..] is highest-degree-first length n.
        let b_coeffs: Vec<f64> = remainder[1..].iter().rev().copied().collect();

        let a: Vec<Vec<f64>> = (0..n)
            .map(|row| {
                if row < n - 1 {
                    (0..n)
                        .map(|col| if col == row + 1 { 1.0 } else { 0.0 })
                        .collect()
                } else {
                    (0..n).map(|col| -a_coeffs[col]).collect()
                }
            })
            .collect();
        let b: Vec<Vec<f64>> = (0..n)
            .map(|row| vec![if row == n - 1 { 1.0 } else { 0.0 }])
            .collect();
        let c: Vec<Vec<f64>> = vec![b_coeffs];
        let d: Vec<Vec<f64>> = vec![vec![d_feedthrough]];

        // e: None always passes StateSpace::new's own validation trivially.
        StateSpace::new(a, b, c, d, None).expect("e=None is always a valid descriptor matrix")
    }

    /// The state of [`Self::to_state_space`]'s realization that a *continuous-time* filter
    /// already settled at output $y_0$ holds — what a netlist's `y0=` (and `kind=pid`'s own
    /// `ic=`) means.
    ///
    /// In controllable canonical form every state is the derivative of the one before it, so an
    /// equilibrium is $x = (x_1, 0, \dots, 0)$, and the last row, $-a_0 x_1 + u_0 = 0$, fixes
    /// the constant input holding it there at $u_0 = a_0 x_1$. The output is then
    /// $y_0 = b_0 x_1 + d\,u_0 = (b_0 + d\,a_0)\,x_1$, and $b_0 + d\,a_0$ is exactly the
    /// normalized numerator's constant term $N(0)$, hence
    ///
    /// $$x_1 = \frac{y_0}{b_0 + d\,a_0}.$$
    ///
    /// Nothing here needs $a_0 \ne 0$: for a transfer function with a pole at the origin (an
    /// integrator, a PID) the holding input is simply $u_0 = 0$, which is the "integrator
    /// pre-loaded, zero error" start a controller wants.
    pub fn settled_state(&self, y0: f64) -> Result<Vec<f64>, SettledStateError> {
        let ss = self.to_state_space();
        let n = ss.states();
        if n == 0 {
            return Err(SettledStateError::NoStates);
        }
        let a0 = -ss.a[n - 1][0];
        let dc_numerator = ss.c[0][0] + ss.d[0][0] * a0;
        if dc_numerator == 0.0 {
            return Err(SettledStateError::ZeroDcNumerator);
        }
        let mut x = vec![0.0; n];
        x[0] = y0 / dc_numerator;
        Ok(x)
    }

    /// The *discrete-time* counterpart of [`Self::settled_state`], for the same realization
    /// stepped as $x[k+1] = A x[k] + B u[k]$.
    ///
    /// Here every state is the one before it delayed by a sample, so an equilibrium has all $n$
    /// states equal, $x_i = \bar{x}$; the last row gives the holding input
    /// $u_0 = (1 + \sum_i a_i)\,\bar{x}$, and the output is
    /// $y_0 = \big(\sum_i b_i + d\,(1 + \sum_i a_i)\big)\,\bar{x}$ — the normalized
    /// numerator evaluated at $z = 1$ — hence
    ///
    /// $$\bar{x} = \frac{y_0}{\sum_i b_i + d\,\big(1 + \sum_i a_i\big)}.$$
    pub fn settled_state_discrete(&self, y0: f64) -> Result<Vec<f64>, SettledStateError> {
        let ss = self.to_state_space();
        let n = ss.states();
        if n == 0 {
            return Err(SettledStateError::NoStates);
        }
        let den_at_one = 1.0 - ss.a[n - 1].iter().sum::<f64>();
        let dc_numerator = ss.c[0].iter().sum::<f64>() + ss.d[0][0] * den_at_one;
        if dc_numerator == 0.0 {
            return Err(SettledStateError::ZeroDcNumerator);
        }
        Ok(vec![y0 / dc_numerator; n])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1/(s+1): a_0=1, b_0=1, d=0 -- matches the textbook single-state realization exactly,
    /// derived by hand before writing this test (see `docs/architecture.md`'s Milestone 5
    /// note).
    #[test]
    fn first_order_lowpass_matches_hand_derived_canonical_form() {
        let tf = TransferFunction::new(vec![1.0], vec![1.0, 1.0]).unwrap();
        let ss = tf.to_state_space();
        assert_eq!(ss.a, vec![vec![-1.0]]);
        assert_eq!(ss.b, vec![vec![1.0]]);
        assert_eq!(ss.c, vec![vec![1.0]]);
        assert_eq!(ss.d, vec![vec![0.0]]);
    }

    /// $G(s) = (s+3)/(s^2+3s+2)$: $a_0 = 2$, $b_0 = 3$, $d = 0$, so $y_0 = 6$ needs
    /// $x_1 = 6/3 = 2$ held by $u_0 = a_0 x_1 = 4$ -- and indeed $G(0)\,u_0 = 1.5 \cdot 4 = 6$.
    /// Checked as a real equilibrium, not just as a formula: the derivative there is zero.
    #[test]
    fn settled_state_is_an_equilibrium_at_the_requested_output() {
        let tf = TransferFunction::new(vec![1.0, 3.0], vec![1.0, 3.0, 2.0]).unwrap();
        let x = tf.settled_state(6.0).unwrap();
        assert_eq!(x, vec![2.0, 0.0]);
        let ss = tf.to_state_space();
        assert_eq!(ss.derivative(&x, &[4.0]), vec![0.0, 0.0]);
        assert_eq!(ss.output(&x, &[4.0]), vec![6.0]);
    }

    /// A pole at the origin: $G(s) = 5/s$, $b_0 = 5$, $a_0 = 0$ -- the holding input is zero and
    /// $x_1 = y_0 / 5$.
    #[test]
    fn settled_state_of_an_integrator_is_held_by_zero_input() {
        let tf = TransferFunction::new(vec![5.0], vec![1.0, 0.0]).unwrap();
        let x = tf.settled_state(10.0).unwrap();
        assert_eq!(x, vec![2.0]);
        let ss = tf.to_state_space();
        assert_eq!(ss.derivative(&x, &[0.0]), vec![0.0]);
        assert_eq!(ss.output(&x, &[0.0]), vec![10.0]);
    }

    /// Biproper, so the feedthrough term matters: $G(s) = (2s+6)/(s+1)$ has $d = 2$,
    /// $b_0 = 6 - 2 = 4$, $a_0 = 1$, $N(0) = 4 + 2 = 6$; $y_0 = 3$ gives $x_1 = 0.5$, $u_0 = 0.5$,
    /// and $y = 4(0.5) + 2(0.5) = 3$.
    #[test]
    fn settled_state_accounts_for_direct_feedthrough() {
        let tf = TransferFunction::new(vec![2.0, 6.0], vec![1.0, 1.0]).unwrap();
        let x = tf.settled_state(3.0).unwrap();
        assert_eq!(x, vec![0.5]);
        assert_eq!(tf.to_state_space().output(&x, &[0.5]), vec![3.0]);
    }

    /// $G(s) = s/(s+1)$ blocks DC entirely; a pure gain has no state.
    #[test]
    fn settled_state_rejects_what_cannot_hold_an_output() {
        let highpass = TransferFunction::new(vec![1.0, 0.0], vec![1.0, 1.0]).unwrap();
        assert_eq!(
            highpass.settled_state(1.0),
            Err(SettledStateError::ZeroDcNumerator)
        );
        let gain = TransferFunction::new(vec![2.0], vec![1.0]).unwrap();
        assert_eq!(gain.settled_state(1.0), Err(SettledStateError::NoStates));
        assert_eq!(
            gain.settled_state_discrete(1.0),
            Err(SettledStateError::NoStates)
        );
    }

    /// $H(z) = (0.5z + 0.25)/(z^2 - 0.5z)$: $H(1) = 0.75/0.5 = 1.5$. With $a = (0, -0.5)$,
    /// $b = (0.25, 0.5)$, $d = 0$: $\bar{x} = 3/0.75 = 4$, held by $u_0 = (1 - 0.5) \cdot 4 = 2$,
    /// and $H(1)\,u_0 = 3$. Checked as a fixed point of the discrete update itself.
    #[test]
    fn discrete_settled_state_is_a_fixed_point_at_the_requested_output() {
        let tf = TransferFunction::new(vec![0.5, 0.25], vec![1.0, -0.5, 0.0]).unwrap();
        let x = tf.settled_state_discrete(3.0).unwrap();
        assert_eq!(x, vec![4.0, 4.0]);
        let ss = tf.to_state_space();
        assert_eq!(ss.discrete_step(&x, &[2.0]), x);
        assert_eq!(ss.output(&x, &[2.0]), vec![3.0]);
    }

    #[test]
    fn rejects_improper_transfer_function() {
        let err = TransferFunction::new(vec![1.0, 0.0, 0.0], vec![1.0, 1.0]).unwrap_err();
        assert_eq!(err, TransferFunctionError::ImproperTransferFunction);
    }
}
