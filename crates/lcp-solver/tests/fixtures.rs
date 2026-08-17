//! Hand-solved LCP fixtures, verified independently of the implementation (see the derivation
//! in each test's comment) before trusting the solver on any circuit. Per this project's
//! verification discipline (see `../../../docs/architecture.md`): numerical self-consistency
//! is not enough, every fixture here has an independently worked complementary solution.

use lcp_solver::{solve, LcpError};

fn assert_close(actual: &[f64], expected: &[f64], tol: f64) {
    assert_eq!(actual.len(), expected.len(), "length mismatch");
    for (i, (a, e)) in actual.iter().zip(expected).enumerate() {
        assert!(
            (a - e).abs() < tol,
            "index {i}: expected {e}, got {a} (tol {tol})"
        );
    }
}

/// q is already nonnegative, so the trivial solution w = q, z = 0 is complementary and Lemke's
/// algorithm should return it without any pivoting (M is irrelevant here as long as it's a
/// valid n x n shape).
#[test]
fn trivial_no_pivot_needed() {
    let m = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
    let q = vec![3.0, 5.0];
    let sol = solve(&m, &q).expect("trivial case must solve");
    assert_close(&sol.w, &[3.0, 5.0], 1e-9);
    assert_close(&sol.z, &[0.0, 0.0], 1e-9);
}

/// n = 1: M = [1], q = [-1]. w = z + q.
/// Branch z = 0 => w = -1, infeasible (w must be >= 0).
/// Branch w = 0 => 0 = z - 1 => z = 1 >= 0, feasible.
/// So the unique complementary solution is w = 0, z = 1.
#[test]
fn single_pivot_scalar_case() {
    let m = vec![vec![1.0]];
    let q = vec![-1.0];
    let sol = solve(&m, &q).expect("must solve");
    assert_close(&sol.w, &[0.0], 1e-9);
    assert_close(&sol.z, &[1.0], 1e-9);
}

/// M = [[2,1],[1,2]] (symmetric positive definite => the monotone LCP has a unique solution),
/// q = [-1,-1]. Solving with w1 = w2 = 0 (the only feasible branch, since any branch leaving a
/// w_i basic forces w_i = q_i = -1 < 0):
///   0 = 2 z1 +  z2 - 1
///   0 =  z1 + 2 z2 - 1
/// => z1 = z2 = 1/3 (both >= 0, so this branch is valid and, by uniqueness for PD M, the only
/// solution).
#[test]
fn positive_definite_case_needs_multiple_pivots() {
    let m = vec![vec![2.0, 1.0], vec![1.0, 2.0]];
    let q = vec![-1.0, -1.0];
    let sol = solve(&m, &q).expect("must solve");
    assert_close(&sol.w, &[0.0, 0.0], 1e-9);
    assert_close(&sol.z, &[1.0 / 3.0, 1.0 / 3.0], 1e-9);
}

/// Degenerate/edge case: M = I, q = [-1, 0]. Index 2 has q_2 = 0 exactly, so w_2 = z_2 = 0
/// simultaneously satisfies complementarity regardless of which one the pivoting path happens
/// to leave basic — a genuine degenerate boundary point (unlike fixtures above where each
/// coordinate has a clear-cut feasible branch). Index 1 still forces w_1 = 0, z_1 = 1 by the
/// same argument as the scalar case above. So the solution's *values* are pinned even though
/// the *basis* at index 2 is not: w = (0, 0), z = (1, 0).
#[test]
fn degenerate_boundary_case() {
    let m = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
    let q = vec![-1.0, 0.0];
    let sol = solve(&m, &q).expect("degenerate case must still solve");
    assert_close(&sol.w, &[0.0, 0.0], 1e-9);
    assert_close(&sol.z, &[1.0, 0.0], 1e-9);
}

/// Sanity check that a solved LCP always satisfies its own defining equations and
/// complementarity, independent of the hand-derived expected values above.
#[test]
fn solutions_satisfy_lcp_definition() {
    let cases: Vec<(Vec<Vec<f64>>, Vec<f64>)> = vec![
        (vec![vec![1.0, 0.0], vec![0.0, 1.0]], vec![3.0, 5.0]),
        (vec![vec![1.0]], vec![-1.0]),
        (vec![vec![2.0, 1.0], vec![1.0, 2.0]], vec![-1.0, -1.0]),
        (vec![vec![1.0, 0.0], vec![0.0, 1.0]], vec![-1.0, 0.0]),
    ];
    for (m, q) in cases {
        let n = q.len();
        let sol = solve(&m, &q).expect("must solve");
        for i in 0..n {
            assert!(sol.w[i] >= -1e-9, "w[{i}] negative");
            assert!(sol.z[i] >= -1e-9, "z[{i}] negative");
            assert!(
                sol.w[i] * sol.z[i] < 1e-6,
                "complementarity violated at {i}"
            );
            let mz_plus_q: f64 = (0..n).map(|j| m[i][j] * sol.z[j]).sum::<f64>() + q[i];
            assert!(
                (sol.w[i] - mz_plus_q).abs() < 1e-6,
                "w = Mz + q violated at {i}: w={}, Mz+q={}",
                sol.w[i],
                mz_plus_q
            );
        }
    }
}

/// A case Lemke's algorithm is known to fail on via ray termination: a negative-definite-style
/// M with no complementary solution reachable from this covering vector. M = [[-1]], q = [-1]:
/// w = -z - 1. Branch z=0 => w=-1 infeasible. Branch w=0 => -z-1=0 => z=-1, also infeasible (z
/// must be >= 0). No complementary solution exists at all for this (M, q) pair, so the solver
/// must report an error rather than fabricate one.
#[test]
fn ray_termination_on_infeasible_problem() {
    let m = vec![vec![-1.0]];
    let q = vec![-1.0];
    let err = solve(&m, &q).expect_err("this LCP has no complementary solution");
    assert_eq!(err, LcpError::RayTermination);
}
