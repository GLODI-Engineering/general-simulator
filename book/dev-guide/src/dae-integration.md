# DAE integration: backward Euler and trapezoidal

## The descriptor DAE shape

Every circuit `general-mna` produces, and every continuous block this project supports
(transfer function, state-space, PID, integrator — see `docs/architecture.md`'s "One descriptor
system for circuit and continuous blocks alike"), is expressed in one shape throughout this
codebase:

$$A x(t) + K \dot x(t) = B u(t)$$

`dae-runtime`'s job, once the LCP fold (see [lcp-formulation.md](lcp-formulation.md)) has picked
out which piecewise-linear segment every device is in for a given step, is turning this
continuous-time DAE into a discrete update from $x_n$ to $x_{n+1}$ — and it implements exactly
two schemes for that, both documented directly on `dae-runtime`'s own `Scheme` enum
(`crates/dae-runtime/src/lib.rs:330-378`).

## Backward Euler

Approximate the derivative with the backward difference $\dot x \approx (x_{n+1}-x_n)/dt$ and
substitute into the DAE at $t_{n+1}$:

$$A x_{n+1} + K\,\frac{x_{n+1}-x_n}{dt} = B u_{n+1} \implies \left(A + \frac{K}{dt}\right) x_{n+1} = B u_{n+1} + \frac{K}{dt} x_n$$

This is `Scheme::BackwardEuler { x_prev, dt }` (`lib.rs:339`), and `fold_and_solve` builds exactly
this effective matrix and RHS (`lib.rs:892-908`): `a_eff = numeric0.a + numeric0.k / dt`,
`u_eff = numeric0.u + (numeric0.k @ x_prev) / dt`. It's only first-order accurate — local error
$O(dt^2)$, verified directly by convergence order rather than by trusting the derivation
(`docs/architecture.md`, "Status": "backward Euler roughly halves error [as dt halves]").

The property that matters for scheme *selection*, though, isn't its order — it's that backward
Euler's derivation places **no requirement on `x_prev`** beyond it being some starting vector.
It doesn't need `x_prev` to already satisfy the circuit's own algebraic constraints (KCL/KVL,
which for a descriptor DAE some rows of are exactly that — the rows of $A$ with a zero row in
$K$). That tolerance for an inconsistent starting point is exactly what makes it the right choice
whenever the previous state can't be trusted to be self-consistent.

## Trapezoidal

Sum the DAE evaluated at $t_n$ and at $t_{n+1}$, and eliminate the derivative using the
trapezoidal relation $\dot x_n + \dot x_{n+1} = (2/dt)(x_{n+1}-x_n)$:

$$\frac{A}{2}(x_n + x_{n+1}) + \frac{K}{dt}(x_{n+1}-x_n) = \frac{B}{2}(u_n + u_{n+1})$$

$$\implies \left(\frac{A}{2} + \frac{K}{dt}\right) x_{n+1} = \frac{B}{2}(u_n+u_{n+1}) + \left(\frac{K}{dt} - \frac{A}{2}\right) x_n$$

This is `Scheme::Trapezoidal { x_prev, dt, prev_diode_raw_ioff }` (`lib.rs:373-377`) — second-order
accurate ($O(dt^3)$ local error, "trapezoidal roughly quarters [error]" per the same
convergence-order check above), matching Xyce/SPICE's own default integration method.

**The derivation's own hidden assumption**: it only holds if summing the DAE at $t_n$
contributes exactly $Bu_n - Ax_n = 0$ — i.e. if $x_n$ *already* satisfies the circuit's own
algebraic constraints exactly. If it doesn't, the derivation above is simply wrong at $t_n$,
not just less accurate. `lib.rs:358-364`'s own doc comment names this directly: "This is the DAE
analog of 'trapezoidal needs consistent initial conditions.'" A plain ODE solver's classic
requirement (a valid, self-consistent starting state) becomes, for a DAE with real algebraic
constraints baked into $A$, a requirement that the *previous solved step* actually satisfied
those constraints — which an arbitrary `x_initial` at the start of a run is not guaranteed to
do, and which a step whose diode segments just changed is not guaranteed to do either (the
algebraic constraint itself changed shape at that instant).

## Why `B u` needs averaging too, once sources stopped being constant

The derivation above needs $(B/2)(u_n + u_{n+1})$, not $B u_{n+1}$ alone — true trapezoidal
accuracy averages the *whole* right-hand side, not just the derivative term. This was invisible
as long as every source and every diode's own resolved current was implicitly constant across a
step ($u_n = u_{n+1}$ trivially), which was true of every netlist this crate handled at first.
Two additions since then each broke that assumption in their own way, and each was fixed the
same way — explicit averaging — rather than by silently reusing the old shortcut:

- **A diode's own resolved current is not constant across a step** — that's the entire point of
  resolving it via the LCP each step. Since the new step's diode current only enters the
  effective RHS as half its full contribution ($(B/2) \cdot \mathrm{raw\_Ioff}_{n+1}$), the LCP
  fold's per-diode coupling coefficients get scaled by `coupling_scale = 0.5` for `Trapezoidal`
  specifically (`lib.rs:909-916`, `956`) — while the *previous* step's already-known current
  contributes its own $(B/2) \cdot \mathrm{raw\_Ioff}_n$ term directly into the RHS via `prev_diode_raw_ioff`
  (`lib.rs:978-991`). This is why `Scheme::Trapezoidal` carries `prev_diode_raw_ioff` as a field
  at all — trapezoidal has a genuine one-step memory that backward Euler doesn't need.
- **A genuinely time-varying source** (`SIN`/`PULSE`/`EXP`/`PWL`/`SFFM`, via
  `general_mna::TransientFunction`) has $u_n \neq u_{n+1}$ for its own column too, not just for
  diode currents. `fold_and_solve` re-evaluates `numeric0.u` a second time at $t_n = t_{n+1} -
  dt$ (`u_prev`, `lib.rs:929-944`) and averages the two — exactly mirroring the diode-current
  averaging above — rather than silently reusing the "constant source" shortcut for a source
  that's no longer constant. When there are neither time-varying sources nor block-driven ones,
  this second evaluation is skipped entirely and `u_prev` is just `numeric0.u.clone()` — a real,
  checked-for optimization for the still-very-common all-constant-source case, not a correctness
  shortcut.

## Scheme selection: why every run starts with, and falls back to, backward Euler

Given the consistent-initial-conditions requirement above, trapezoidal is only safe to use when
the previous step is known to have produced a self-consistent `x_prev`. `dae-runtime`'s transient
loops therefore force backward Euler in exactly the situations where that's not guaranteed
(`crates/dae-runtime/src/block_graph.rs:2416`, `:2503`; `step_control::lte_attempt`,
`crates/dae-runtime/src/step_control.rs:134-151`):

1. **The very first step of a run** (`step_index == 0`). An arbitrary `x_initial` — all zero by
   default, or a netlist's own `ic=` values — has no reason to already satisfy the circuit's
   algebraic constraints exactly.
2. **Any step whose gate state just changed** (`gate_changed`). An ideal switch flipping is a
   structural rebuild of the whole system (see
   [switch-model-ideal-switch.md](switch-model-ideal-switch.md)) — the algebraic constraints
   themselves are different on the two sides of the flip, so nothing about the previous step's
   consistency with the *old* constraints says anything about the *new* ones.
3. **Any step during an active ringing cooldown** (`ringing_cooldown > 0`) — covered in full in
   [ringing-and-fallback.md](ringing-and-fallback.md), but load-bearing here too: a single
   corrective backward-Euler step isn't enough to fully re-establish "consistent" in the sense
   trapezoidal's own derivation needs, so the fallback persists for a few steps rather than one.

Two more triggers are only detectable *after* actually attempting a trapezoidal trial step, since
they depend on what that trial itself resolves — `step_with_fallback`
(`crates/dae-runtime/src/lib.rs:626-682`) tries trapezoidal first, then discards the result and
redoes the step with backward Euler if either fires:

- **A diode's resolved segment differs from the previous step's** (`classify_segments`,
  `lib.rs:555-569`, compared via `segments_changed`). Exactly the same principle as trigger 2
  above, but for the LCP's own discrete choice rather than an externally-commanded gate: a
  segment change is itself a change to which algebraic constraints are active, so the
  just-computed trapezoidal trial (which assumed the *old* segment set's constraints held at
  `x_prev`) can't be trusted.
- **Ringing** is detected in the trapezoidal trial's own result — see the next chapter.

The adaptive-step path (`step_control::lte_attempt`, `step_control.rs:118-219`) makes exactly the
same decision, for exactly the same reasons, on top of its own local-truncation-error retry loop
— `force_backward_euler` there is driven by the identical three up-front conditions, and
`segments_changed`/`ringing` are checked against the same trapezoidal trial the error estimate
itself is computed from (`step_control.rs:176-191`).

This is the numerical-stability payoff of the whole "backward Euler is *always* available and
*always* safe, trapezoidal is faster-converging but conditionally valid" split: rather than
trying to patch trapezoidal's derivation to tolerate an inconsistent starting point (which would
mean re-deriving a different, lower-accuracy scheme in disguise), this crate just falls back to
the one scheme that was designed for exactly that tolerance, and resumes the more accurate one
once consistency is re-established.
