//! First end-to-end proof that PWL devices + `lcp-solver` reproduce a hand-derived circuit
//! solution with no Newton-Raphson anywhere. Topology mirrors the illustrative circuit from
//! the Sandia PCNR paper (Aadithya, Keiter, Mei 2020 — the same paper Xyce's own authors wrote
//! about the problems with SPICE-style voltage limiting; see
//! `internal-archive/explanations/xyce/voltage-limiting-current-status.md`), with its
//! exponential diodes replaced by our PWL diodes:
//!
//! ```text
//!   e1 (= Vsrc, ideal source to ground) --+-- D1 --+
//!                                         |        |
//!                                         +-- D2 --+-- e2 --[ R ]-- ground
//! ```
//!
//! D1, D2 both anode-at-`e1`, in parallel, so both see the same voltage `V = e1 - e2`.
//!
//! ## Hand derivation (independent of the code below, checked before it was written)
//!
//! Diodes: `g_breakdown = 0`, `v_breakdown = -100` (irrelevant at these operating points, kept
//! nonzero-width only for the model to be well-formed), `g_off = 0` (ideal, zero leakage),
//! `g_on = 1` for both; `v_th1 = 1 V`, `v_th2 = 2 V`. Source `e1 = 5 V`, `R = 1 ohm`.
//!
//! With `g_off = 0` everywhere, KCL at `e2` is `I_D1(V) + I_D2(V) = e2 / R = (e1 - V) / R`.
//! Guessing both diodes end up forward-conducting (checked after the fact):
//! `I_D1 = V - 1`, `I_D2 = V - 2`, so `(V-1) + (V-2) = 5 - V` => `3V = 8` => `V = 8/3`.
//! Check: `8/3 > 2 > 1`, so the forward-conducting guess for both diodes is self-consistent.
//! `e2 = 5 - 8/3 = 7/3`. `I_D1 = 8/3 - 1 = 5/3`. `I_D2 = 8/3 - 2 = 2/3`.
//! (`I_D1 + I_D2 = 7/3 = e2 / R`, checks out.)
//!
//! The code below builds the *same* circuit as a Linear Complementarity Problem — using each
//! diode's canonical `(z1, z2)` decomposition (`pwl_devices::IdealDiodeCanonical`) folded through
//! the one-node KCL equation — and checks that `lcp_solver::solve` recovers `V`, `e2`, `I_D1`,
//! `I_D2` matching the derivation above, without any Newton iteration.

use lcp_solver::solve;
use pwl_devices::IdealDiode;

/// Build `(M, q)` for "N diodes in parallel between a fixed source `e1` and a node `e2`, with
/// a resistor `R` from `e2` to ground" — folding each diode's canonical decomposition through
/// KCL at `e2` to eliminate `V = e1 - e2` in favor of the diodes' `z` variables. Variable
/// ordering: `[z1_0, z2_0, z1_1, z2_1, ...]`, one `(z1, z2)` pair per diode.
///
/// This is deliberately specific to this one-node topology (a hand-rolled linear solve, per
/// this project's Milestone 2 scope) — folding *arbitrary* circuit topologies through PWL
/// devices generically is `dae-runtime`'s job, a later milestone.
fn build_lcp(diodes: &[IdealDiode], e1: f64, r: f64) -> (Vec<Vec<f64>>, Vec<f64>) {
    let n = diodes.len();
    let canon: Vec<_> = diodes.iter().map(|d| d.canonical()).collect();

    // KCL at e2: sum_k I_Dk(V) = e2 / R = (e1 - V) / R, with I_Dk(V) = g_off_k * V +
    // delta_on_k * z2_k - delta_br_k * z1_k. Solve for V as an affine function of every z:
    //   V = V0 + sum_k (delta_br_k / denom) * z1_k - (delta_on_k / denom) * z2_k
    let denom: f64 = canon.iter().map(|c| c.g_off).sum::<f64>() + 1.0 / r;
    let v0 = (e1 / r) / denom;

    // coupling[j] is the coefficient of z-variable j (interleaved z1/z2 per diode) in V.
    let mut coupling = vec![0.0; 2 * n];
    for (k, c) in canon.iter().enumerate() {
        coupling[2 * k] = c.delta_br / denom;
        coupling[2 * k + 1] = -c.delta_on / denom;
    }

    // w1_k = V - v_breakdown_k + z1_k, w2_k = v_th_k - V + z2_k. Substitute V(z) in and read
    // off M, q row by row.
    let mut m = vec![vec![0.0; 2 * n]; 2 * n];
    let mut q = vec![0.0; 2 * n];
    for (k, c) in canon.iter().enumerate() {
        let row_w1 = 2 * k;
        q[row_w1] = v0 - c.v_breakdown;
        m[row_w1].copy_from_slice(&coupling);
        m[row_w1][2 * k] += 1.0; // + z1_k term

        let row_w2 = 2 * k + 1;
        q[row_w2] = c.v_th - v0;
        for (dst, src) in m[row_w2].iter_mut().zip(coupling.iter()) {
            *dst = -src;
        }
        m[row_w2][2 * k + 1] += 1.0; // + z2_k term
    }
    (m, q)
}

#[test]
fn two_pwl_diodes_reproduce_hand_derived_operating_point() {
    let d1 = IdealDiode::new(0.0, -100.0, 0.0, 1.0, 1.0);
    let d2 = IdealDiode::new(0.0, -100.0, 0.0, 2.0, 1.0);
    let e1 = 5.0;
    let r = 1.0;

    let (m, q) = build_lcp(&[d1, d2], e1, r);
    let sol = solve(&m, &q).expect("this circuit's LCP has a solution");

    // z = [z1_0, z2_0, z1_1, z2_1]
    let z1_1 = sol.z[0];
    let z2_1 = sol.z[1];
    let z1_2 = sol.z[2];
    let z2_2 = sol.z[3];

    let tol = 1e-9;
    assert!(
        z1_1.abs() < tol,
        "D1 should not be in breakdown: z1_1={z1_1}"
    );
    assert!(
        z1_2.abs() < tol,
        "D2 should not be in breakdown: z1_2={z1_2}"
    );
    assert!(
        (z2_1 - 5.0 / 3.0).abs() < 1e-6,
        "z2_1 (D1 above threshold) = {z2_1}"
    );
    assert!(
        (z2_2 - 2.0 / 3.0).abs() < 1e-6,
        "z2_2 (D2 above threshold) = {z2_2}"
    );

    // Reconstruct V, e2, and each diode's current the same way the physical circuit would,
    // and cross-check against the diode's own (independently written) `current()` function.
    let v = e1 - (z2_1 + z2_2); // from the KCL fold in build_lcp with these zero-g_off diodes
    let e2 = e1 - v;
    let i_d1 = d1.current(v);
    let i_d2 = d2.current(v);

    assert!((v - 8.0 / 3.0).abs() < 1e-6, "V = {v}, expected 8/3");
    assert!((e2 - 7.0 / 3.0).abs() < 1e-6, "e2 = {e2}, expected 7/3");
    assert!(
        (i_d1 - 5.0 / 3.0).abs() < 1e-6,
        "I_D1 = {i_d1}, expected 5/3"
    );
    assert!(
        (i_d2 - 2.0 / 3.0).abs() < 1e-6,
        "I_D2 = {i_d2}, expected 2/3"
    );

    // KCL sanity: total diode current must equal the resistor current e2/R.
    assert!(((i_d1 + i_d2) - e2 / r).abs() < 1e-6);
}

/// A second operating point where only D1 conducts (source lowered below D2's threshold),
/// to confirm the LCP correctly resolves *different* devices into *different* segments, not
/// just the "both forward" case above. With only D1's threshold (1 V) reachable:
/// I_D1(V) = e2/R = (e1-V)/R => (V-1) = (1.5-V) => 2V = 2.5 => V = 1.25, e2 = 0.25.
/// Check: 1.25 > 1 (D1 forward) and 1.25 < 2 (D2 still off) — self-consistent.
#[test]
fn only_first_diode_conducts_at_lower_source_voltage() {
    let d1 = IdealDiode::new(0.0, -100.0, 0.0, 1.0, 1.0);
    let d2 = IdealDiode::new(0.0, -100.0, 0.0, 2.0, 1.0);
    let e1 = 1.5;
    let r = 1.0;

    let (m, q) = build_lcp(&[d1, d2], e1, r);
    let sol = solve(&m, &q).expect("this circuit's LCP has a solution");

    let z2_1 = sol.z[1];
    let z2_2 = sol.z[3];
    assert!((z2_1 - 0.25).abs() < 1e-6, "z2_1 = {z2_1}, expected 0.25");
    assert!(z2_2.abs() < 1e-9, "D2 must be off (z2_2 == 0): got {z2_2}");

    let v = e1 - (z2_1 + z2_2);
    assert!((v - 1.25).abs() < 1e-6, "V = {v}, expected 1.25");
    assert!((d1.current(v) - 0.25).abs() < 1e-6);
    assert!(d2.current(v).abs() < 1e-9, "D2 off => zero current");
}
