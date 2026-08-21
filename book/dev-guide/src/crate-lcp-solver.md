# `lcp-solver`: Lemke's algorithm

*(Skeleton — outline below; not yet written.)*

## What goes here
- The LCP defined precisely: find $w, z \ge 0$ with $w = Mz + q$ and $w^\top z = 0$.
- Lemke's algorithm at a level a reader can actually follow: the covering-vector/artificial-
  variable setup, the pivoting rule, termination (a solution, or ray termination = infeasible).
- The one real bug caught during development worth keeping as a case study: the initial pivot
  row must minimize $q_i/d_i$ for a *general* covering vector, not raw $q_i$ — they only
  coincide when $d$ is all-ones.
- What "trusted standalone" verification looked like here specifically: which hand-solved
  fixtures, and why each one (trivial, single-pivot, multi-pivot, degenerate, infeasible) earns
  its place in the suite rather than being redundant.

## Source material to adapt from
- `crates/lcp-solver/src/tableau.rs` and `lemke.rs` doc comments.
- `crates/lcp-solver/tests/fixtures.rs` — six tests, each already documented with what property
  it independently verifies.
- `elspice-pwl`'s own journal, Milestone 1 entry, for the pivot-rule bug account.
