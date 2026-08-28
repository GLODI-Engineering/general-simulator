//! A descriptor state-space system `E * dx/dt = A*x + B*u`, `y = C*x + D*u` — the same
//! `A x + K dx/dt = B u` shape `general-mna` uses for circuits, with `K = E` (a standard
//! "Descriptor State-Space" block, `E dx/dt = Ax + Bu`, is textually identical to this
//! convention — see `docs/architecture.md`). Plain `E = I` is the ordinary (non-descriptor)
//! state-space case.
//!
//! Matrices are plain row-major `Vec<Vec<f64>>` here rather than `general_mna::Matrix`: block
//! parameters (gains, pole/zero locations, PID coefficients) are ordinary known numbers at
//! model-build time, not symbolic netlist parameters, so there is no `Expression` layer to
//! carry — wiring a block's *evaluated* `(A, K, B)` into a circuit's global descriptor system
//! is `dae-runtime`'s job in a later milestone.

#[derive(Debug, Clone, PartialEq)]
pub struct StateSpace {
    /// `n x n` dynamics matrix.
    pub a: Vec<Vec<f64>>,
    /// `n x m` input matrix (`m` = number of inputs).
    pub b: Vec<Vec<f64>>,
    /// `p x n` output matrix (`p` = number of outputs).
    pub c: Vec<Vec<f64>>,
    /// `p x m` feedthrough matrix.
    pub d: Vec<Vec<f64>>,
    /// `n x n` descriptor matrix. `None` means the identity (ordinary state-space); an explicit
    /// non-identity `e` is the "Descriptor State-Space" case.
    pub e: Option<Vec<Vec<f64>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateSpaceError {
    /// The descriptor matrix `e` is singular — `E * dx/dt = A*x + B*u` has no unique `dx/dt`
    /// for a generic `(A, x, B, u)`, so [`StateSpace::derivative`] could never solve it.
    /// Checked once here, at construction, rather than on every single step's own
    /// [`StateSpace::derivative`] call (which stays infallible as a result) — the same
    /// validate-once-at-construction convention [`crate::Vco`]/[`crate::Pmsm`]/
    /// [`crate::Hysteresis`]/[`crate::Pid`] all use for their own invariants.
    NonInvertibleDescriptorMatrix,
}

impl StateSpace {
    /// Validates `e` (if given) is invertible, then returns the assembled system. Every
    /// [`StateSpace::derivative`] call downstream relies on this having already been checked —
    /// see [`StateSpaceError::NonInvertibleDescriptorMatrix`]'s own doc comment.
    pub fn new(
        a: Vec<Vec<f64>>,
        b: Vec<Vec<f64>>,
        c: Vec<Vec<f64>>,
        d: Vec<Vec<f64>>,
        e: Option<Vec<Vec<f64>>>,
    ) -> Result<Self, StateSpaceError> {
        if let Some(e) = &e {
            let n = e.len();
            if dense_solve(e, &vec![0.0; n]).is_err() {
                return Err(StateSpaceError::NonInvertibleDescriptorMatrix);
            }
        }
        Ok(StateSpace { a, b, c, d, e })
    }

    pub fn states(&self) -> usize {
        self.a.len()
    }

    pub fn inputs(&self) -> usize {
        self.b.first().map_or(0, |row| row.len())
    }

    pub fn outputs(&self) -> usize {
        self.c.len()
    }

    /// `y = C*x + D*u`.
    pub fn output(&self, x: &[f64], u: &[f64]) -> Vec<f64> {
        (0..self.outputs())
            .map(|row| {
                let cx: f64 = (0..self.states())
                    .map(|col| self.c[row][col] * x[col])
                    .sum();
                let du: f64 = (0..self.inputs())
                    .map(|col| self.d[row][col] * u[col])
                    .sum();
                cx + du
            })
            .collect()
    }

    /// `dx/dt` at the given `(x, u)`, solving `E * dx/dt = A*x + B*u` for `dx/dt` when a
    /// non-identity descriptor matrix `e` is present (via the same dense Gauss-Jordan solve
    /// `dae-runtime` uses — small, hand-checkable, `faer` deferred until it's actually needed;
    /// see that crate's `linsolve` module doc for the same reasoning applied here).
    pub fn derivative(&self, x: &[f64], u: &[f64]) -> Vec<f64> {
        let rhs: Vec<f64> = (0..self.states())
            .map(|row| {
                let ax: f64 = (0..self.states())
                    .map(|col| self.a[row][col] * x[col])
                    .sum();
                let bu: f64 = (0..self.inputs())
                    .map(|col| self.b[row][col] * u[col])
                    .sum();
                ax + bu
            })
            .collect();
        match &self.e {
            None => rhs,
            // Safe: StateSpace::new already rejected a singular `e` at construction time.
            Some(e) => dense_solve(e, &rhs).expect("StateSpace::new guarantees e is invertible"),
        }
    }

    /// Advances one RK4 step of size `dt`, holding `u` constant over the step (zero-order
    /// hold — matches how a discrete-timestep circuit/controller simulation would drive this
    /// block in `dae-runtime`'s eventual timestep loop). Returns the new state; call
    /// [`StateSpace::output`] separately for `y` at the new state.
    pub fn rk4_step(&self, x: &[f64], u: &[f64], dt: f64) -> Vec<f64> {
        let add = |a: &[f64], b: &[f64], scale: f64| -> Vec<f64> {
            a.iter().zip(b).map(|(ai, bi)| ai + scale * bi).collect()
        };
        let k1 = self.derivative(x, u);
        let k2 = self.derivative(&add(x, &k1, dt / 2.0), u);
        let k3 = self.derivative(&add(x, &k2, dt / 2.0), u);
        let k4 = self.derivative(&add(x, &k3, dt), u);
        (0..x.len())
            .map(|i| x[i] + (dt / 6.0) * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]))
            .collect()
    }

    /// One step of a genuinely **discrete-time** system: `x[i+1] = A*x[i] + B*u[i]`, plain
    /// linear recursion, no RK4, no `dt`, no numerical integration at all — the discrete-domain
    /// counterpart to [`Self::rk4_step`]. `A`/`B` here are already discrete-domain matrices,
    /// given directly by the caller (this method does no continuous-to-discrete conversion of
    /// its own) — see `book/dev-guide/src/discrete-time-blocks.md`. `dae-runtime` calls this
    /// once per declared sample period instead of once per circuit step, the same zero-order-
    /// hold accumulator `cscript`/`pyblock` already use. Ignores `self.e` entirely — a
    /// descriptor matrix has no meaning for a plain discrete recursion (there is no `dx/dt` to
    /// solve `E` against here).
    pub fn discrete_step(&self, x: &[f64], u: &[f64]) -> Vec<f64> {
        (0..self.states())
            .map(|row| {
                let ax: f64 = (0..self.states())
                    .map(|col| self.a[row][col] * x[col])
                    .sum();
                let bu: f64 = (0..self.inputs())
                    .map(|col| self.b[row][col] * u[col])
                    .sum();
                ax + bu
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SingularMatrix;

fn dense_solve(a: &[Vec<f64>], b: &[f64]) -> Result<Vec<f64>, SingularMatrix> {
    let n = a.len();
    let mut augmented: Vec<Vec<f64>> = a
        .iter()
        .zip(b)
        .map(|(row, &rhs)| {
            let mut r = row.clone();
            r.push(rhs);
            r
        })
        .collect();

    for pivot_col in 0..n {
        let pivot_row = (pivot_col..n)
            .max_by(|&l, &r| {
                augmented[l][pivot_col]
                    .abs()
                    .total_cmp(&augmented[r][pivot_col].abs())
            })
            .expect("range is nonempty");
        if augmented[pivot_row][pivot_col].abs() <= 1e-12 {
            return Err(SingularMatrix);
        }
        augmented.swap(pivot_row, pivot_col);
        let pivot = augmented[pivot_col][pivot_col];
        for col in pivot_col..=n {
            augmented[pivot_col][col] /= pivot;
        }
        for row in 0..n {
            if row == pivot_col {
                continue;
            }
            let factor = augmented[row][pivot_col];
            if factor == 0.0 {
                continue;
            }
            for col in pivot_col..=n {
                augmented[row][col] -= factor * augmented[pivot_col][col];
            }
        }
    }
    Ok((0..n).map(|row| augmented[row][n]).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn e_none_is_always_valid() {
        let ss = StateSpace::new(
            vec![vec![-1.0]],
            vec![vec![1.0]],
            vec![vec![1.0]],
            vec![vec![0.0]],
            None,
        );
        assert!(ss.is_ok());
    }

    #[test]
    fn a_singular_descriptor_matrix_is_rejected_not_a_panic() {
        // A 2x2 all-zero e: E*dx/dt = A*x+B*u has no unique dx/dt for a generic RHS.
        let e = Some(vec![vec![0.0, 0.0], vec![0.0, 0.0]]);
        let ss = StateSpace::new(
            vec![vec![-1.0, 0.0], vec![0.0, -1.0]],
            vec![vec![1.0], vec![1.0]],
            vec![vec![1.0, 0.0]],
            vec![vec![0.0]],
            e,
        );
        assert_eq!(ss, Err(StateSpaceError::NonInvertibleDescriptorMatrix));
    }

    #[test]
    fn an_invertible_descriptor_matrix_is_accepted() {
        let e = Some(vec![vec![2.0, 0.0], vec![0.0, 3.0]]);
        let ss = StateSpace::new(
            vec![vec![-1.0, 0.0], vec![0.0, -1.0]],
            vec![vec![1.0], vec![1.0]],
            vec![vec![1.0, 0.0]],
            vec![vec![0.0]],
            e,
        );
        assert!(ss.is_ok());
    }

    #[test]
    fn discrete_step_is_a_plain_linear_recursion_no_dt_involved() {
        // x[i+1] = 0.5*x[i] + 2*u[i] -- a first-order decaying discrete system, hand-computed
        // for a few steps starting from x=1, u=1 held constant.
        let ss = StateSpace::new(
            vec![vec![0.5]],
            vec![vec![2.0]],
            vec![vec![1.0]],
            vec![vec![0.0]],
            None,
        )
        .unwrap();
        let mut x = vec![1.0];
        x = ss.discrete_step(&x, &[1.0]);
        assert_eq!(x, vec![0.5 * 1.0 + 2.0 * 1.0]); // 2.5
        x = ss.discrete_step(&x, &[1.0]);
        assert_eq!(x, vec![0.5 * 2.5 + 2.0 * 1.0]); // 3.25
                                                    // output y = C*x + D*u = x here (C=[1], D=[0]).
        assert_eq!(ss.output(&x, &[1.0]), vec![3.25]);
    }
}
