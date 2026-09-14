# Why not Newton-Raphson

## The standard approach, and where it comes from

Every SPICE-family circuit simulator — ngspice, Xyce, HSPICE, and other commercial SPICE
derivatives — solves a nonlinear
DC or transient operating point the same way: Newton-Raphson on the KCL/KVL-plus-device-model
system `g(x) = 0`. Xyce's own math formulation document writes the iteration not as the scalar
textbook form $x_{n+1} = x_n - f(x_n)/f'(x_n)$, but split into a linear solve and an update:

$$J(x_k)\,\Delta x = -g(x_k), \qquad x_{k+1} = x_k + \Delta x$$

These are algebraically identical — $\Delta x = -J^{-1}g(x_k)$ substituted back in gives the
one-line inverse form — but the split matters at circuit scale. A circuit with $N$ nodes gives
an $N\times N$ Jacobian; forming $J^{-1}$ explicitly costs $O(N^3)$ and produces a dense matrix
even when $J$ itself is sparse, whereas solving $J\,\Delta x = -g$ via sparse LU never forms the
inverse at all (an internal write-up on Xyce's Newton-Raphson formulation,
grounded against `Xyce_Math_Formulation.md:319-323`). None of that is this project's own
concern, though — no crate here runs a Newton iteration — but it's worth having the real shape
in mind before reading why this codebase doesn't use it.

## Why plain Newton-Raphson doesn't converge on real device curves

The trouble isn't the linear-algebra cost. It's convergence. A diode's exponential I-V curve

$$I = I_S\left(e^{V/V_T} - 1\right)$$

has a derivative that grows exponentially with $V$. A Newton step computed from a bad starting
guess can propose a swing in $V$ large enough that the next evaluation of $I_S e^{V/V_T}$
overflows, or lands so far from the linearization point that the local Jacobian is meaningless
— the iteration diverges instead of converging. This isn't a corner case; it's routine for any
circuit with real semiconductor devices and a not-already-close starting guess, which is most
of the first iteration of most simulations.

The production fix is **voltage limiting**: cap how far a device's own terminal voltage is
allowed to move in a single Newton step (schemes like `pnjlim`), so the exponential stays
numerically sane while the iteration still makes progress. Xyce's own developers call it
"indispensable" — not a legacy wart kept around for compatibility, but a technique every
production simulator still actively maintains (an internal write-up on Xyce's voltage-limiting
status cites Xyce's 2023-2024 release notes documenting ongoing
`$limit`-related fixes for BSIMSOI convergence, as of this writing).

## Why limiting, despite being indispensable, is also a known wart

Limiting works by modifying the Newton update *outside* the clean `g(x) = 0` abstraction, and
that has real, documented costs — not folklore, but the conclusion of a peer-reviewed Sandia
paper written by Xyce's own math-formulation author. Karthik Aadithya, Eric R. Keiter, and Ting
Mei's "Predictor/Corrector Newton-Raphson" (Sandia SAND2018-5689C, Springer 2020; ingested into
an internal reference archive) walks a concrete
worked example — two diodes in series with a resistor, driven by a DC source — to show exactly
what goes wrong when two devices share a node:

- Each device independently computes its own clamped intermediate voltage using the
  **previous** iteration's value. `g(x)` is therefore no longer a pure function of `x`: evaluating
  it twice at the same `x` can give two different answers, because the result silently depends on
  the path the iteration took to get there.
- Two devices sharing the same physical node voltage can disagree about what the "limited"
  value of that shared voltage even is, since each one limits independently and locally, with no
  shared notion of the node's own history.
- This reproduces, mechanically, the three specific incompatibilities the Xyce Math Formulation
  document itself lists for limiting: the right-hand side is modified beyond just `-g`, the
  resulting update is inconsistent under naive scalar step-size scaling, and the whole thing is
  path-dependent — hysteretic under backtracking, since undoing and retrying a step doesn't
  reproduce the same clamped intermediate values it produced the first time.

The PCNR paper's own proposed fix — treating each limited voltage as an explicit extra unknown
in the MNA system, split into a prediction phase (ordinary unlimited Newton) and a correction
phase that applies the limit consistently across every device sharing a node — is itself
telling: it takes a dedicated research paper, with a nontrivial Schur-complement elimination
scheme to keep the extra unknowns affordable, to make voltage limiting behave like a
well-defined function of `x` again. That is not a small implementation detail; it's a structural
consequence of grafting a clamp onto an iteration whose entire convergence theory assumes a
smooth, iteration-history-independent `g`. As of the sources checked, PCNR remains a research
proposal, not Xyce's shipped default — voltage limiting, warts included, is still what actually
runs in production.

## This project's premise instead

If every device's curve is piecewise-linear rather than smooth-and-exponential, the circuit is
**exactly linear** within any one fixed combination of active segments — no approximation, no
linearization error, because there's no curvature to linearize away in the first place. That
reframes the whole problem: instead of "iterate Newton on continuous device physics, clamping
voltage swings to keep it from diverging," the question becomes "resolve, once per timestep,
which discrete combination of segments is active across every piecewise-linear device in the
circuit simultaneously."

That's a fundamentally different kind of numerical problem — not a smaller Newton iteration, but
no Newton iteration at all. It's a **Linear Complementarity Problem**: for a circuit with $n$
piecewise-linear devices, exactly one segment of every device's curve is active at any genuine
operating point, and "segment $i$ active" can be encoded as a nonnegative slack variable
complementary to a nonnegative "how far past this segment's boundary" variable. Solved in one
finite pivoting pass (Lemke's algorithm), this determines every device's active segment
simultaneously and self-consistently — the same rigorous approach commercial
piecewise-linear circuit solvers use, with no clamping, no path-dependence, and no
iteration-history-dependent `g`, because `g` was never iterated in the first place.

The mechanism itself — how a diode's three-segment curve becomes two complementarity pairs, and
how a whole circuit's worth of piecewise-linear devices folds into one `(M, q)` LCP — is the
subject of the next chapter, [The generic Thevenin/LCP fold](lcp-formulation.md); this chapter
is motivation only. `docs/architecture.md` in this repository's own root is the original,
still-current statement of this same premise, of which this chapter and the next are an expanded
telling.
