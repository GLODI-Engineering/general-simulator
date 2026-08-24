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

    #[test]
    fn rejects_improper_transfer_function() {
        let err = TransferFunction::new(vec![1.0, 0.0, 0.0], vec![1.0, 1.0]).unwrap_err();
        assert_eq!(err, TransferFunctionError::ImproperTransferFunction);
    }
}
