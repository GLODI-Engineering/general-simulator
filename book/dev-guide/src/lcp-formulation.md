# The generic Thevenin/LCP fold

*(Skeleton — outline below; not yet written.)*

## What goes here
- The canonical diode decomposition: fixed reference conductance `g_off` (so the linear system
  $A_0$ is genuinely fixed, independent of segment) plus a per-instance current source term
  `Ioff`, carrying all the nonlinearity.
- Full derivation, with real math notation:
  $$x(z) = x_0 + \sum_j w_j \cdot \mathrm{raw\_Ioff}_j, \qquad V_k(z) = v_{0,k} + \sum_j \gamma_{kj}\cdot \mathrm{raw\_Ioff}_j$$
  where $x_0 = A_0^{-1} u_0$ and $w_j = A_0^{-1} B_{:,j}$ — walk through why this makes every
  diode's terminal voltage an *affine* function of every diode's own complementarity variables.
- Assembling $(M, q)$: two complementarity pairs per diode (`w1,z1` guarding breakdown,
  `w2,z2` guarding the forward threshold), the cross-coupling `gamma[k][j]` term explained as
  "how much diode $j$'s unit current moves diode $k$'s own terminal voltage through the shared
  linear network."
- One fully worked small example (2 diodes) computed by hand, matching an existing test fixture
  — readers should be able to check the book's own arithmetic against
  `crates/pwl-devices/tests/two_diode_circuit.rs`.
- Close with: this is why MOSFET *channel* switching can't reuse the same trick (forward-link
  to `switch-model-mosfets.md` rather than re-arguing it here).

## Source material to adapt from
- `crates/dae-runtime/src/lib.rs`'s own module doc comment (lines ~1-25) — this *is* the
  derivation, already written; the job here is expanding it with the worked numeric example and
  real KaTeX rendering, not re-deriving from scratch.
- `crates/dae-runtime/src/lib.rs`'s `fold_and_solve` function body as the "here's the code that
  implements exactly this" companion.
