//! The dense simplex-style tableau Lemke's algorithm pivots on.
//!
//! Variables are indexed `0..n` for `w`, `n..2n` for `z`, and `2n` for the artificial `z0`.
//! Columns `0..=2n` hold coefficients; column `2n + 1` holds the right-hand side.
//!
//! Row `i`, with basic variable `b_i`, always satisfies
//! `b_i = rhs_i - sum_{nonbasic j} T[i][j] * x_j`
//! so a basic variable's current value is simply `rhs_i` once the tableau is in reduced form
//! relative to the current basis (every basic column is a unit vector).

pub const EPS: f64 = 1e-9;

pub struct Tableau {
    pub n: usize,
    /// Row-major, `n` rows by `2n + 2` columns (see module docs for the column layout).
    pub rows: Vec<Vec<f64>>,
    /// `basis[i]` is the variable index currently basic in row `i`.
    pub basis: Vec<usize>,
}

impl Tableau {
    pub fn z0_col(&self) -> usize {
        2 * self.n
    }

    pub fn rhs_col(&self) -> usize {
        2 * self.n + 1
    }

    /// Build the initial tableau for `w - M z - d z0 = q`, with `w` basic (`w_i` in row `i`).
    pub fn new(m: &[Vec<f64>], q: &[f64], d: &[f64]) -> Self {
        let n = q.len();
        let mut rows = vec![vec![0.0; 2 * n + 2]; n];
        for i in 0..n {
            rows[i][i] = 1.0; // w_i column
            for j in 0..n {
                rows[i][n + j] = -m[i][j]; // z_j column
            }
            rows[i][2 * n] = -d[i]; // z0 column
            rows[i][2 * n + 1] = q[i]; // rhs
        }
        let basis = (0..n).collect();
        Tableau { n, rows, basis }
    }

    /// Complement of variable `v`: `w_i <-> z_i`. `z0` (index `2n`) has no complement and must
    /// never be passed here.
    pub fn complement(&self, v: usize) -> usize {
        debug_assert!(v != self.z0_col(), "z0 has no complement");
        if v < self.n {
            v + self.n
        } else {
            v - self.n
        }
    }

    /// Gauss-Jordan pivot: variable `entering` becomes basic in `row`, replacing whatever was
    /// basic there. Returns the variable that left the basis.
    pub fn pivot(&mut self, row: usize, entering: usize) -> usize {
        let leaving = self.basis[row];
        let ncols = self.rows[row].len();

        let pivot_val = self.rows[row][entering];
        debug_assert!(pivot_val.abs() > EPS, "pivoting on a near-zero element");
        for c in 0..ncols {
            self.rows[row][c] /= pivot_val;
        }

        for r in 0..self.n {
            if r == row {
                continue;
            }
            let factor = self.rows[r][entering];
            if factor.abs() <= EPS {
                continue;
            }
            for c in 0..ncols {
                self.rows[r][c] -= factor * self.rows[row][c];
            }
        }

        self.basis[row] = entering;
        leaving
    }

    /// Minimum-ratio test restricted to rows with a strictly positive coefficient in
    /// `entering`'s column. Ties are broken toward the row whose basic variable is `z0` (to
    /// terminate as soon as a complementary solution is available), then toward the
    /// smallest-indexed basic variable (Bland's rule, to guard against cycling).
    pub fn ratio_test(&self, entering: usize) -> Option<usize> {
        let rhs = self.rhs_col();
        let mut best_row: Option<usize> = None;
        let mut best_ratio = f64::INFINITY;
        for i in 0..self.n {
            let coef = self.rows[i][entering];
            if coef <= EPS {
                continue;
            }
            let ratio = self.rows[i][rhs] / coef;
            match best_row {
                None => {
                    best_row = Some(i);
                    best_ratio = ratio;
                }
                Some(r) => {
                    if ratio < best_ratio - EPS {
                        best_row = Some(i);
                        best_ratio = ratio;
                    } else if ratio < best_ratio + EPS {
                        // Tie: prefer evicting z0, else the smaller basic-variable index.
                        let prefer_current_z0 = self.basis[i] == self.z0_col();
                        let prefer_best_z0 = self.basis[r] == self.z0_col();
                        let switch = (prefer_current_z0 && !prefer_best_z0)
                            || (prefer_current_z0 == prefer_best_z0
                                && self.basis[i] < self.basis[r]);
                        if switch {
                            best_row = Some(i);
                            best_ratio = ratio;
                        }
                    }
                }
            }
        }
        best_row
    }
}
