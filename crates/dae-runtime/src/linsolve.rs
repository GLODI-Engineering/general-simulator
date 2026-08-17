//! A small dense linear solve (Gauss-Jordan, partial pivoting), used to fold a generic
//! circuit's linear part into an LCP. Deliberately hand-rolled rather than pulling in `faer`
//! yet: at this milestone every circuit is small and dense, and a solver this size is easy to
//! read and trust outright. `docs/architecture.md` already commits to `faer` for the
//! sparse/dense numeric layer once circuit size or sparsity actually make it worth the
//! dependency — revisit then, not before.

use elspice_mna::Matrix;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SingularMatrix {
    pub pivot_column: usize,
}

/// Solves `a * x = b` for `x`. `a` must be square.
pub fn dense_solve(a: &Matrix<f64>, b: &[f64]) -> Result<Vec<f64>, SingularMatrix> {
    let n = a.rows();
    assert_eq!(a.cols(), n, "matrix must be square");
    assert_eq!(b.len(), n, "rhs length must match matrix order");

    let mut augmented = vec![vec![0.0; n + 1]; n];
    for row in 0..n {
        for col in 0..n {
            augmented[row][col] = a[(row, col)];
        }
        augmented[row][n] = b[row];
    }

    for pivot_col in 0..n {
        let pivot_row = (pivot_col..n)
            .max_by(|&l, &r| {
                augmented[l][pivot_col]
                    .abs()
                    .total_cmp(&augmented[r][pivot_col].abs())
            })
            .expect("range is nonempty");
        if augmented[pivot_row][pivot_col].abs() <= 1e-12 {
            return Err(SingularMatrix {
                pivot_column: pivot_col,
            });
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
    fn solves_a_hand_checkable_system() {
        // [2 1; 1 3] x = [5; 10]  =>  x = [1, 3] (2*1+1*3=5, 1*1+3*3=10).
        let a = Matrix::from_vec(2, 2, vec![2.0, 1.0, 1.0, 3.0]).unwrap();
        let x = dense_solve(&a, &[5.0, 10.0]).unwrap();
        assert!((x[0] - 1.0).abs() < 1e-9);
        assert!((x[1] - 3.0).abs() < 1e-9);
    }

    #[test]
    fn reports_singular_matrix() {
        let a = Matrix::from_vec(2, 2, vec![1.0, 1.0, 2.0, 2.0]).unwrap();
        assert!(dense_solve(&a, &[1.0, 2.0]).is_err());
    }
}
