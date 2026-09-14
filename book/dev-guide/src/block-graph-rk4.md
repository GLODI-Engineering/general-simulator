# Dynamic blocks: one RK4 per block, wired by causality

## One integrator per block, never one fused ODE

Every dynamic block — `Pid`, `StateSpace`, `TransferFunction` (via `continuous_blocks::StateSpace::
rk4_step`) and `Pmsm` (via its own bespoke `step()`) — integrates *independently*, once per
circuit step, holding its own state in a separate field inside its own `BlockState` entry
(`block_states: Vec<BlockState>`, one slot per `BlockInstance`, built once at the start of a run
and mutated in place thereafter). Stepping one block never touches another block's state array.
There is no single fused ODE spanning the whole block graph the way a block-diagram simulation
tool's continuous states are sometimes integrated jointly by one solver call. The dispatch that
does this stepping is `evaluate_blocks`' own match arms:

```rust
(BlockKind::TransferFunction(_), BlockState::Dynamic { state_space, x }) => {
    let u = require_all_scalar(&block.name, &input_vals)?;
    *x = state_space.rk4_step(x, &u, dt);
    SignalValue::Scalar(state_space.output(x, &u)[0])
}
(BlockKind::StateSpace(_), BlockState::Dynamic { state_space, x }) => {
    let u = flatten(&input_vals);
    // ... length check against state_space.inputs() ...
    *x = state_space.rk4_step(x, &u, dt);
    // ...
}
```

(`crates/dae-runtime/src/block_graph.rs:1046-1077`), and for `Pmsm`:

```rust
(BlockKind::Pmsm { output_names, .. }, BlockState::Pmsm { pmsm, x }) => {
    let flat = flatten(&input_vals);
    *x = pmsm.step(*x, flat[0], flat[1], flat[2], dt);
    // ...
}
```

(`crates/dae-runtime/src/block_graph.rs:1226-1234`). Each of these calls is entirely local: it
reads this block's own current state array, this step's already-resolved input values, and
writes back only this block's own state.

## Why this is correct, not just simpler

The correctness argument rests on exactly the machinery `block-graph-causality.md` already
establishes: `evaluate_blocks` runs every block in `topological_order`, so by the time any block
is stepped, every `Signal::Block` input it needs is already a concrete, fully-resolved number for
*this* step — never a live reference into some other block's still-in-progress integration. The
2026-08-21 14:21 journal entry (the durable source this chapter expands) states the three-part
argument plainly:

1. Every dynamic block's state lives in its own `BlockState` slot, never touched by stepping a
   different block.
2. `rk4_step`/`Pmsm::step` hold the input vector `u` *fixed* across all four RK4 stages — `k1`
   through `k4` all call `self.derivative(x_trial, u)` with the same `u`
   (`crates/continuous-blocks/src/state_space.rs:113-124`; `crates/continuous-blocks/
   src/pmsm.rs:129-134`) — a zero-order hold over one circuit step, which is exactly what a real
   digital controller's discrete update already does: read inputs once, integrate forward, done.
   It is not an approximation of a continuously-coupled system; it is the correct discrete-time
   model of a sampled digital controller.
3. Because nothing needs a block's own *mid-step* trial state (`k1`/`k2`/`k3`) — only its already
   -fixed inputs — there is no possible entanglement between two different blocks' RK4 stages the
   way there would be if two separate blocks each needed the *other's* current-step output to
   compute their own derivative. That would be a genuine same-step algebraic loop
   (`block-graph-cycles.md`), not something any amount of per-block integrator cleverness could
   resolve — the fix there is always `prev:`, never a fancier integrator.

(`docs/journal/2026-08.md:1300-1316`.)

## `Pmsm`: the case worth walking through

`Pmsm` is the one block in this family whose *own* internal dynamics are genuinely coupled —
worth singling out because it's the case where a fused-integration instinct might seem tempting.
Its module doc comment is explicit about why it can't be a `StateSpace` at all:

> the back-EMF/cross-coupling terms (`omega_e * l_q * iq`, `omega_e * l_d * id`) and the
> electromagnetic torque (`(l_d - l_q) * id * iq`) are *bilinear* — products of two of this
> block's own states (speed and current), not linear in the state vector — so this genuinely
> cannot be expressed as the descriptor system `A x + K dx/dt = B u` every other dynamic block in
> this crate compiles to.
>
> — `crates/continuous-blocks/src/pmsm.rs:8-12`

This is the same underlying reason this whole project avoids Newton-Raphson iteration on device
physics elsewhere (nonlinearity defeats a fixed linear-fragment representation), here showing up
one level up the stack: not in a circuit device's own $I$-$V$ curve, but in a control block's own
state dynamics. `Pmsm::derivative` (`crates/continuous-blocks/src/pmsm.rs:109-118`) computes the
full nonlinear vector field — all four bilinear/coupling terms — in one function:

```rust
fn derivative(&self, x: [f64; 4], vd: f64, vq: f64, t_load: f64) -> [f64; 4] {
    let [id, iq, omega_m, _theta_e] = x;
    let omega_e = self.pole_pairs * omega_m;
    let did = (vd - self.r_s * id + omega_e * self.l_q * iq) / self.l_d;
    let diq = (vq - self.r_s * iq - omega_e * self.l_d * id - omega_e * self.lambda_pm) / self.l_q;
    let domega = (self.torque(id, iq) - t_load - self.friction * omega_m) / self.inertia;
    let dtheta = omega_e;
    [did, diq, domega, dtheta]
}
```

and `Pmsm::step` (`crates/continuous-blocks/src/pmsm.rs:123-135`) calls this same `derivative`
four times — once per RK4 stage, each at its own trial state `x_trial`, all with `vd`/`vq`/
`t_load` held fixed — exactly the pattern `StateSpace::rk4_step` uses. Because the *entire*
nonlinear field, coupling terms included, is evaluated inside each of those four calls, RK4's own
intermediate stages see the true coupled dynamics at each trial point, the same way any ordinary
nonlinear-ODE integrator correctly handles a coupled system when given the whole vector field at
once. The module doc comment draws the direct contrast with what would go wrong under a
different design: "there is no cross-block algebraic loop to resolve the way there would be if
`id`/`iq` dynamics were split across two separate blocks that each needed the other's current-step
output" (`crates/continuous-blocks/src/pmsm.rs:17-19`). If `id` and `iq` were instead computed by
two separate `StateSpace` blocks, each needing the *other* block's just-computed current to
evaluate its own bilinear coupling term this step, that would be exactly the same-step algebraic
loop `block-graph-cycles.md` describes — unsolvable by any per-block RK4, because the two blocks'
derivatives would each depend on data the other hasn't produced yet. `Pmsm` sidesteps this
entirely by not being decomposed at the block-graph level at all: the coupling lives *inside* one
block's own vector field, evaluated by one integrator, not *across* the block graph's own
same-step dependency edges.

`crates/continuous-blocks/src/pmsm.rs:189-232`'s own `full_coupled_system_converges_at_fourth_
order` test is the concrete verification that this actually works as claimed: run with a nonzero
initial speed and nonzero `vd`/`vq`/`t_load` (so every bilinear term is genuinely engaged, not
accidentally zero), halving `dt` from a 64-step run to a 128-step run should shrink RK4's error
against a 4096-step reference by roughly $2^4 = 16\times$ — the test asserts the ratio falls in
`8.0..32.0`, confirming the integrator retains its full fourth-order convergence rate even with
the coupling active, not silently degraded to some lower effective order by the bilinear terms.

## Wired into the same causal graph as every other block — not a separate loop

None of the above makes `Pmsm` (or any other dynamic block) a disconnected integration island.
Its three inputs (`vd`, `vq`, `t_load`) are ordinary `Signal`s, resolved by the same `resolve`
closure and flattened by the same `flatten` helper (`crates/dae-runtime/src/block_graph.rs:89-94`)
every other input-bundling block (`StateSpace`, `CoordinateTransform`, `cscript`) uses, and its
four outputs (`id`, `iq`, `omega_m`, `theta_e`) are written into the very same `outputs` map
`evaluate_blocks` builds for every other block this step, immediately readable by any downstream
block via `Signal::Block("PMSM_NAME")` (or the appropriate `output_names` entry) exactly like a
`Const` or a `Sum`. `evaluate_blocks` calls `pmsm.step(...)` from inside the same `for &i in
order` loop, at `i`'s own position in the `topological_order`-derived sequence, that steps every
other block — there is no separate pass, no separate scheduler, no separate timestep for `Pmsm`.
The block-graph module's own doc comment names the resulting overall structure precisely:

> Sampled-data co-simulation: every block reads the *previous* circuit step's measurement, the
> whole graph is evaluated once per circuit step (in the causal order [`topological_order`]
> derives ...), and the result decides gate states before the circuit step itself is solved.
>
> — `crates/dae-runtime/src/block_graph.rs:27-30`

and the 2026-08-21 14:21 journal entry gives the precise nesting this produces: "the block graph
and the circuit are two separate discrete-time systems, each reading the other's state at fixed
sample points, and *within* the block graph, each dynamic block is its own further-independent
discrete-time system too — not one jointly-solved continuous whole" (`docs/journal/
2026-08.md:1326-1329`). `Pmsm`'s own RK4 step is the innermost layer of that nesting: one more
independent discrete-time system, causally wired into the same `Signal::Block` graph as
everything else, stepped once per circuit step at the position `topological_order` assigns it —
not a special case bolted on beside the mechanism the rest of this chapter's siblings describe,
but one more instance of it.
