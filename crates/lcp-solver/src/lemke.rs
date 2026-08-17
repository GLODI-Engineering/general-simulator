use crate::tableau::{Tableau, EPS};

#[derive(Debug, Clone, PartialEq)]
pub struct Solution {
    pub w: Vec<f64>,
    pub z: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LcpError {
    /// Lemke's algorithm reached a "secondary ray": the entering variable's column had no
    /// positive entry to pivot on, so the almost-complementary path runs off to infinity
    /// instead of finding a solution. This is a valid outcome for some `(M, q)` (e.g. `M` not
    /// copositive-plus) and does not necessarily mean the LCP is infeasible in general — only
    /// that Lemke's path failed to certify a solution.
    RayTermination,
    /// The pivot count exceeded the safety cap without terminating. With Bland's-rule-style
    /// tie-breaking this should not happen on well-posed circuit-sized problems; treat it as a
    /// bug or a pathological input if it does.
    MaxIterationsExceeded,
}

/// Solve `w = M z + q`, `w, z >= 0`, `w . z = 0` via Lemke's algorithm.
///
/// `M` must be `n x n` (row-major, `M[row][col]`), `q` length `n`. Uses the all-ones covering
/// vector `d`, the standard choice unless a problem-specific covering vector is known to do
/// better.
pub fn solve(m: &[Vec<f64>], q: &[f64]) -> Result<Solution, LcpError> {
    let n = q.len();
    assert_eq!(m.len(), n, "M must have n rows");
    for row in m {
        assert_eq!(row.len(), n, "M must have n columns");
    }
    let d = vec![1.0; n];
    solve_with_covering_vector(m, q, &d)
}

/// Same as [`solve`] but with an explicit covering vector `d` (must be strictly positive for
/// Lemke's algorithm's feasibility argument to hold).
pub fn solve_with_covering_vector(
    m: &[Vec<f64>],
    q: &[f64],
    d: &[f64],
) -> Result<Solution, LcpError> {
    let n = q.len();
    if n == 0 {
        return Ok(Solution {
            w: vec![],
            z: vec![],
        });
    }

    // Trivial case: q already feasible with z = 0, no pivoting (and no z0) needed at all.
    if q.iter().all(|&qi| qi >= -EPS) {
        return Ok(Solution {
            w: q.iter().map(|&qi| qi.max(0.0)).collect(),
            z: vec![0.0; n],
        });
    }

    for &di in d {
        assert!(di > 0.0, "covering vector must be strictly positive");
    }
    let mut t = Tableau::new(m, q, d);

    // Drive z0 in: pivot on the row minimizing q_i / d_i, so that after this one pivot every
    // row's RHS becomes q_i - (q_r0/d_r0) * d_i >= 0 (this is why d must be strictly positive
    // and why the ratio, not the raw q_i, is what must be minimized in general).
    let r0 = (0..n)
        .min_by(|&a, &b| (q[a] / d[a]).partial_cmp(&(q[b] / d[b])).unwrap())
        .unwrap();
    let z0_col = t.z0_col();
    let mut leaving = t.pivot(r0, z0_col);

    let max_iters = 200 * n + 1000;
    for _ in 0..max_iters {
        let entering = t.complement(leaving);
        let row = match t.ratio_test(entering) {
            Some(r) => r,
            None => return Err(LcpError::RayTermination),
        };
        let left = t.pivot(row, entering);
        if left == z0_col {
            return Ok(extract(&t));
        }
        leaving = left;
    }
    Err(LcpError::MaxIterationsExceeded)
}

fn extract(t: &Tableau) -> Solution {
    let n = t.n;
    let mut w = vec![0.0; n];
    let mut z = vec![0.0; n];
    let rhs = t.rhs_col();
    for row in 0..n {
        let var = t.basis[row];
        let val = t.rows[row][rhs].max(0.0);
        if var < n {
            w[var] = val;
        } else if var < 2 * n {
            z[var - n] = val;
        }
        // var == z0_col: z0 is nonbasic at solution (0), nothing to record.
    }
    Solution { w, z }
}
