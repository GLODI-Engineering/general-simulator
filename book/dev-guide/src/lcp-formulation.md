# The generic Thevenin/LCP fold

This chapter is a walkthrough of `crates/dae-runtime/src/lib.rs`'s own module-level doc comment
(lines 1-25 as of this writing) — the real derivation already lives there, in the crate itself,
as the authoritative source; this chapter exists to unpack the notation for a reader who hasn't
already internalized it, and to work one small example by hand.

## The canonical decomposition, one diode at a time

`pwl_devices::IdealDiode` models a three-segment diode — reverse breakdown below `v_breakdown`,
near-zero leakage between `v_breakdown` and `v_th`, forward conduction above `v_th`
(`crates/pwl-devices/src/ideal_diode.rs:1-21`). The direct piecewise formula (`IdealDiode::current`,
`ideal_diode.rs:42-52`) evaluates whichever segment `v` falls into. That's the physically obvious
representation, but it's useless for circuit assembly: which formula applies depends on `v`,
which is exactly the unknown the circuit solve is trying to find.

The Chua-Lin canonical decomposition (`IdealDiode::canonical`, `ideal_diode.rs:54-75`) rewrites
the same curve as one fixed linear term plus two `max(0, ...)` corrections, anchored on the
*leakage* segment (the one passing through the origin at slope `g_off`):

$$I(v) = g_{\text{off}}\,v + \delta_{\text{on}} z_2 - \delta_{\text{br}} z_1$$

$$z_1 = \max(0,\ v_{\text{breakdown}} - v), \qquad z_2 = \max(0,\ v - v_{\text{th}})$$

where `delta_br = g_breakdown - g_off` and `delta_on = g_on - g_off`. This is an *identity*, not
an approximation — `crates/pwl-devices/src/ideal_diode.rs`'s own
`canonical_matches_direct_piecewise_evaluation` test checks it against the direct piecewise
formula at every segment and both breakpoints, and it always matches to `1e-9`. What's bought by
rewriting it this way is that `z1`/`z2` are no longer "which segment is this" — they're two
nonnegative numbers, at most one of which is nonzero at any true operating point (a diode isn't
simultaneously below breakdown and above threshold), which is exactly the shape a
complementarity condition needs: $z_1 \ge 0$, $z_2 \ge 0$, and each is driven to zero whenever
the corresponding "how far past this boundary" slack is strictly positive.

## Why fixing the conductance at `g_off` keeps the linear system fixed

`general-mna` stamps each diode as a **fixed** conductance `{name}_G` (set to `g_off`, the
diode's canonical reference slope) plus a per-instance Norton current-source symbol
`{name}_Ioff`. That word "fixed" is the entire reason this approach works: no matter which
segment a diode ends up resolved into, the diode's contribution to the circuit's own linear
system — the conductance stamped into $A$ — never changes. Only the current-source term
changes, and current sources enter a linear system additively, on the right-hand side, not as a
structural change to $A$ itself.

Concretely: call the linear system with every diode fixed at `g_off` and every `Ioff = 0` the
baseline system $A_0 x_0 = u_0$. Because $A_0$ doesn't depend on which diode is in which segment,
it can be **factored once per timestep** and reused for every diode's own sensitivity — this is
the entire "Thevenin" idea (`fold_and_solve`, `crates/dae-runtime/src/lib.rs:846` onward, computes
`x0` and every `w_j` against the one shared `a_eff` built from that fixed system).

Contrast this with what happens if a device's *conductance itself* changes between segments (an
ideal switch's channel going from open to a plain resistor, say): the topology of $A$ genuinely
changes, and there is no fixed $A_0$ to build this whole trick on. That's exactly why an ideal
switch's channel state can't reuse this mechanism — see
[switch-model-ideal-switch.md](switch-model-ideal-switch.md).

## Every diode's voltage as an affine function of every diode's `z`

Since the system is linear in the `Ioff` terms, superposition applies directly. Let
$x_0 = A_0^{-1} u_0$ be the baseline operating point with every diode's current source at zero,
and let $w_j = A_0^{-1} B_{:,j}$ be how much a unit "raw `Ioff`" on diode $j$ moves *every*
unknown in the circuit ($B_{:,j}$ is that diode's own input column in the system's `B` matrix —
`fold_and_solve` computes this as `dense_solve(&a_eff, &b_col)`, `lib.rs:993`). Then for any
combination of diode currents $\mathrm{raw\_Ioff}_j$:

$$x(z) = x_0 + \sum_j w_j \cdot \mathrm{raw\_Ioff}_j, \qquad \mathrm{raw\_Ioff}_j = \delta_{\text{on},j}\,z_{2,j} - \delta_{\text{br},j}\,z_{1,j}$$

and, reading off the two terminal nodes $p_k$/$n_k$ of diode $k$:

$$V_k(z) = v_{0,k} + \sum_j \gamma_{kj}\cdot \mathrm{raw\_Ioff}_j, \qquad \gamma_{kj} = w_j[p_k] - w_j[n_k]$$

`gamma[k][j]` is computed exactly this way in code (`lib.rs:1020-1028`, `get(&j.w, k.p) -
get(&j.w, k.n)`) — the comment right above it states the physical reading directly: "how much
diode $j$'s unit current moves diode $k$'s own terminal voltage." The diagonal term
$\gamma_{kk}$ is a diode's effect on its own voltage; every off-diagonal term is genuine
cross-coupling through the shared linear network — two diodes on the same node move each other's
terminal voltage even though neither one "knows" about the other directly. This is what makes
the eventual LCP solve simultaneous across every diode in the circuit rather than one-at-a-time:
$M$'s off-diagonal blocks *are* that coupling.

## Assembling `(M, q)`

Each diode contributes two complementarity pairs — `w1, z1` guarding the breakdown boundary,
`w2, z2` guarding the forward threshold. Substituting $V_k(z)$ into the guard equations from the
canonical decomposition:

$$w_{1,k} = V_k - v_{\text{breakdown},k} + z_{1,k} \ge 0, \qquad w_{2,k} = v_{\text{th},k} - V_k + z_{2,k} \ge 0$$

and expanding $V_k(z)$ in terms of every $z_{1,j}, z_{2,j}$ gives exactly $w = Mz + q$: the
`q` entries are $v_{0,k} - v_{\text{breakdown},k}$ and $v_{\text{th},k} - v_{0,k}$ (the baseline
slack at each guard, before any diode current is applied), and the `M` entries are the
$\gamma_{kj}$ coupling terms scaled by each diode $j$'s own `delta_br`/`delta_on`, with a `+1`
on each row's own diagonal complementarity term (`lib.rs:1030-1049` builds exactly this, row by
row). Solving this LCP (Lemke's algorithm, `crates/lcp-solver`) for `z` that satisfies
$w,z \ge 0$, $w \cdot z = 0$ picks out one self-consistent combination of active segments across
every diode simultaneously, in one finite pivoting pass — no iteration, because there is nothing
to iterate: $M$ and $q$ are fixed numbers once $A_0$, $x_0$, and every $w_j$ are computed, and
the LCP solve is exact, not an approximation refined step by step.

## A fully worked example: two diodes in parallel

`crates/pwl-devices/tests/two_ideal_diode_circuit.rs` hand-builds exactly this fold for one
specific, simple topology — two diodes in parallel between a fixed source and a shared node,
through a resistor to ground — deliberately chosen to mirror the illustrative circuit from the
Sandia PCNR paper this project's own "why not Newton-Raphson" motivation cites (see
[architecture-overview.md](architecture-overview.md)), with its exponential diodes replaced by
PWL ones:

```text
  e1 (source, 5V) --+-- D1 (v_th=1V) --+
                     |                  |
                     +-- D2 (v_th=2V) --+-- e2 --[ R=1ohm ]-- ground
```

Both diodes are anode-at-`e1`, ideal (`g_breakdown = g_off = 0`), `g_on = 1`, differing only in
threshold: `v_th1 = 1V`, `v_th2 = 2V`. There's only one non-source node, `e2`, and both diodes
see the same voltage `V = e1 - e2`, so KCL at `e2` is `I_D1(V) + I_D2(V) = e2/R = (e1-V)/R`.

**By hand** (`two_ideal_diode_circuit.rs:16-27`): guess both diodes end up forward-conducting
(checked after the fact). Then `I_D1 = V - 1`, `I_D2 = V - 2`, so

$$(V-1) + (V-2) = 5 - V \implies 3V = 8 \implies V = 8/3$$

Check: $8/3 \approx 2.667 > 2 > 1$, so both diodes really are above their own thresholds —
the forward-conducting guess is self-consistent. From there, $e_2 = 5 - 8/3 = 7/3$,
$I_{D1} = 8/3 - 1 = 5/3$, $I_{D2} = 8/3 - 2 = 2/3$ (and indeed $I_{D1}+I_{D2} = 7/3 = e_2/R$).

**Through the LCP fold**, with `g_off = 0` for both diodes (so $A_0$ is just the bare resistor
network, independent of any diode entirely), the test builds `(M, q)` by the same substitution
this chapter just walked through, specialized to this one-node topology
(`two_ideal_diode_circuit.rs:45-80`), and hands it to `lcp_solver::solve`. The solver returns
$z_{2,1} = 5/3$, $z_{2,2} = 2/3$, both $z_1 = 0$ — i.e. both diodes resolved into their forward segment,
with $z_2$ equal to exactly the hand-derived diode currents above (not a coincidence: with
`g_off = 0` and both diodes forward-conducting, $\mathrm{raw\_Ioff} = \Delta_{on} \cdot z_2 = 1 \cdot z_2$ *is* the
diode current). Reconstructing $V = e_1 - (z_{2,1} + z_{2,2}) = 5 - 7/3 = 8/3$ and $e_2 = e_1 - V = 7/3$
matches the hand derivation to `1e-6`, and a second fixture in the same file
(`only_first_diode_conducts_at_lower_source_voltage`) lowers the source to 1.5V and confirms the
LCP correctly resolves D2 into its *off* segment ($z_{2,2} = 0$) while D1 stays forward-conducting —
proof the fold distinguishes genuinely different devices ending up in genuinely different
segments, not just the everything-forward case. A reader can run
`cargo test -p pwl-devices --test two_ideal_diode_circuit` directly against this repository to
check the book's own arithmetic above, rather than trusting it by assertion.

## What this fold can't do

The entire mechanism above depends on one thing: every segment of a diode sharing the *same*
reference conductance `g_off`, so swapping segments only ever shifts a current source at fixed
topology. An ideal switch's channel going from open to `r_on` is not that kind of change — it's
the literal appearance of a conductance path, a structural change to $A_0$ itself, which breaks
the fixed-$A_0$ assumption this whole chapter is built on. That's a genuinely different problem,
covered in [switch-model-ideal-switch.md](switch-model-ideal-switch.md) rather than re-argued
here.
