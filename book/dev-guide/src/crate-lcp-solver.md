# `lcp-solver`: Lemke's algorithm

The numerical primitive the whole project stands on: given `M` (`n x n`) and `q` (`n`), find
`w, z in R^n` with

$$w = M z + q, \qquad w \ge 0, \qquad z \ge 0, \qquad w \cdot z = 0 \quad (w_i z_i = 0 \text{ for every } i).$$

`crates/lcp-solver/src/lib.rs` defines this and nothing else: "This crate has no knowledge of
circuits, devices, or `general-mna`; it only implements the LCP numerics, so it can be tested
and trusted in isolation first." That last clause is the entire reason the crate exists as its
own workspace member — `AGENTS.md` calls it "the highest-risk, least-familiar numerical piece"
and requires it to be verified standalone before anything builds on top. What
`general-simulator` does with an LCP (deciding, once per timestep, which linear segment of
every PWL device is active, replacing Newton-Raphson + voltage limiting) is
`docs/architecture.md`'s story; this chapter is the solver itself.

## The algorithm, followable

`src/lemke.rs::solve_with_covering_vector` implements Lemke's method over the dense
simplex-style tableau in `src/tableau.rs`. The tableau's variable layout: indices `0..n` are
`w`, `n..2n` are `z`, and `2n` is the artificial variable `z0`. Row `i`, with basic variable
`b_i`, always satisfies $b_i = \mathrm{rhs}_i - \sum_{j \notin \text{basis}} T_{ij}\, x_j$, so once the tableau is
reduced (every basic column a unit vector), a basic variable's current value is just its
row's RHS.

The run of the algorithm, as the code comments spell it out:

1. **Trivial exit.** If `q >= 0` already, `w = q, z = 0` is complementary and no pivoting is
   needed — returned immediately (the `q.iter().all(|&qi| qi >= -EPS)` fast path).
2. **Non-finite check.** A NaN/Inf anywhere in `M`, `q`, or `d` returns
   `LcpError::NonFiniteInput` *before* any pivoting, because `NaN.partial_cmp(_)` is `None` and
   the pivot-row selection would otherwise panic on a `partial_cmp().unwrap()`.
3. **Drive `z0` in.** The initial tableau is `w - M z - d z0 = q` with `w` basic. The first
   pivot enters `z0`, and the row it enters on is the one **minimizing `q_i / d_i`** — not raw
   `q_i`. Minimizing the ratio is what guarantees every RHS becomes
   `q_i - (q_r0/d_r0) * d_i >= 0` after that one pivot; it also requires `d` to be strictly
   positive (asserted). `solve` itself just calls this with the all-ones covering vector,
   `d = [1; n]`; `solve_with_covering_vector` is the escape hatch for a general positive `d`.
4. **Complementary pivots.** Each iteration enters the complement of the variable that just
   left the basis (`t.complement(leaving)` — `w_i`'s complement is `z_i` and vice versa), ratio-tests
   for the leaving row, and stops the moment `z0` leaves the basis: the resulting basic
   solution is complementary, and `extract` reads it back off the RHS column (`z0` itself is
   nonbasic at the solution, hence 0, and records nothing).
5. **Termination.** If the ratio test finds no positive pivot entry, the almost-complementary
   path runs off to infinity: `LcpError::RayTermination`. This is a *valid outcome* for some
   `(M, q)` — it does not prove infeasibility in general, only that Lemke's path failed to
   certify a solution. A cap of `200*n + 1000` pivots guards the other failure mode
   (`LcpError::MaxIterationsExceeded`), "treat it as a bug or a pathological input" if it fires.

## The pivot-rule bug, as a case study

The one real bug caught during development is preserved in
`docs/journal/2026-08.md`'s Milestone 1 entry (2026-08-17): **the initial pivot row must
minimize `q_i / d_i`, not raw `q_i`.** For the default all-ones covering vector the two
coincide (`d_i = 1` for every row), so the bug is invisible on every all-ones test; it only
shows for a genuinely general `d`, where picking the most-negative `q_i` can put a row with a
small `d_i` into the basis and leave a *negative* RHS behind — breaking the very feasibility
argument the artificial-variable setup exists to establish. It was found in review, not by a
failing test, and `solve_with_covering_vector` is the public API that makes the general case
reachable at all — which is why its existence is tested explicitly (`nan_in_m_is_a_clean_error_via_solve_with_covering_vector`
and the trivial-feasible smoke test both go through it).

## What "trusted standalone" looked like here

Six tests in `crates/lcp-solver/tests/fixtures.rs`, each with an **independently hand-solved**
expected solution in its own doc comment (the file's module comment states the discipline:
"numerical self-consistency is not enough"). Each fixture earns its place by exercising one
distinct property:

- `trivial_no_pivot_needed` — `q >= 0`; the solution is `w = q, z = 0` with no pivoting at
  all, and `M` is irrelevant. Locks in the fast path.
- `single_pivot_scalar_case` — `n = 1`, `M = [1]`, `q = [-1]`; the only feasible branch is
  `w = 0, z = 1`. The smallest possible non-trivial exercise of the complementary-pivot loop.
- `positive_definite_case_needs_multiple_pivots` — `M = [[2,1],[1,2]]` (SPD, hence unique
  solution), `q = [-1,-1]`, hand-solved to `z = [1/3, 1/3]`. Proves the algorithm walks more
  than one pivot.
- `degenerate_boundary_case` — `q = [-1, 0]`: index 2 sits exactly on the degeneracy
  ($w_2 = z_2 = 0$ satisfies complementarity regardless of basis). The *values* are pinned
  even though the *basis* is not — a genuinely different failure mode from the clean-cut
  cases.
- `solutions_satisfy_lcp_definition` — re-checks every fixture's answer against the LCP's own
  defining equations, independent of the hand-derived expected values: a cross-check that
  catches e.g. an extract bug that still produced plausible-looking numbers.
- `ray_termination_on_infeasible_problem` — `M = [[-1]], q = [-1]` has **no** complementary
  solution at all (both branches force a negative variable). The solver must report
  `LcpError::RayTermination`, not fabricate an answer — the error path is a first-class
  behavior, not an afterthought.

Plus two NaN fixtures (`nan_in_q_is_a_clean_error_not_a_panic`,
`nan_in_m_is_a_clean_error_via_solve_with_covering_vector`): before the explicit
`NonFiniteInput` check, an overflowing upstream device parameter would have reached a
`partial_cmp().unwrap()` panic inside pivot-row selection — verified now to fail cleanly.

## Cross-references

- `lcp-formulation.md` — how a circuit's diode guards become this crate's `(M, q)`.
- `crate-pwl-devices.md` — where the `max(0, ...)` `z` decomposition comes from.
- `crate-dae-runtime.md` — where `solve` is actually called.

## Source material this was adapted from

- `crates/lcp-solver/src/lib.rs` — the LCP definition and the standalone-crate statement.
- `crates/lcp-solver/src/lemke.rs` — `solve`/`solve_with_covering_vector`, the fast path,
  the pivot-row selection, the error variants.
- `crates/lcp-solver/src/tableau.rs` — the tableau layout and reduction convention.
- `crates/lcp-solver/tests/fixtures.rs` — the six hand-solved fixtures, each test's own
  derivation comment.
- `docs/journal/2026-08.md` — the Milestone-1 entry (2026-08-17), the pivot-rule bug account.
