# `continuous-blocks`: the block library

The crate that turns block-diagram elements into numbers: transfer function, state-space,
PID, voltage-controlled oscillator, hysteresis comparator, coordinate transforms, logic, and
the stateless math ops. `crates/continuous-blocks/src/lib.rs`'s module doc states the
organizing idea this library exists to serve: **each block does one job and is meant to be
wired to the others by the caller** — an error signal is a `Sum` block's output, a
frequency-modulated PWM carrier is `Pid -> Gain -> Vco`, "not a single fused 'closed loop'
function." The netlist-facing documentation of every block kind is the Component Reference
(generated from `general-mna/src/block_graph.rs`); this chapter is about the crate's own
shape.

## Deliberately standalone, and what that buys

`lib.rs` is explicit: this crate has **no dependency on `general-mna`, `pwl-devices`, or
`dae-runtime`**. The reason is not isolationism — it is the verification discipline from
`AGENTS.md` applied at the crate boundary. Block parameters (gains, pole/zero locations, PID
coefficients) are ordinary known numbers at model-build time, not symbolic netlist
parameters, so there is no `Expression` layer to carry (that is why
`state_space.rs` uses plain row-major `Vec<Vec<f64>>` rather than `general_mna::Matrix`); and
"every block here is verified against a hand-derived result on its own before anything wires
it into a whole circuit's global system — the same incremental-verification discipline
`lcp-solver` and `pwl-devices` were built with." Wiring a compiled block's `(A, K, B)` into a
circuit's global descriptor system is a later (and now, in `dae-runtime`'s block graph,
realized) step that sits on top of this crate, not inside it.

The unifying math: every *dynamic* block compiles to a `StateSpace` — the same descriptor-DAE
shape circuits use, `A x + K dx/dt = B u` with `K` here called `e` for descriptor systems
("A standard 'Descriptor State-Space' block, `E dx/dt = Ax + Bu`, is textually identical to
this convention — no translation needed at all"). Why that shape matters project-wide is
`docs/architecture.md`'s "One descriptor system for circuit and continuous blocks alike";
`block-graph-descriptor.md` carries the block-graph half.

## Module map

- **The descriptor-DAE-compiling blocks** — `state_space.rs` (`E dx/dt = Ax + Bu,
  y = Cx + Du`, plain `E = I` the ordinary case), `transfer_function.rs` (`N(s)/D(s)`,
  coefficients highest-degree first, required proper: nonzero-leading `D`,
  `deg N <= deg D` — realized via controllable canonical form), `pid.rs` (parallel PID with
  filtered derivative, $C(s) = K_p + \frac{K_i}{s} + \frac{K_d N s}{s + N}$ — the
  pure-derivative term alone is non-causal, so every real PID filters it; the struct is "the
  compiled gain set only," with the anti-windup behavior living at the block-graph evaluation
  layer), and `discrete_pid.rs` (the discrete counterpart, realized directly from its own
  block diagram — three parallel branches summed — rather than by converting one combined
  `z`-domain transfer function; see `discrete-time-blocks.md` for why, and for the
  derivative-filter-always-Forward-Euler rule: Forward Euler is the only method of the three
  with no direct feedthrough, so the filter's own feedback topology would be an unresolvable
  same-step algebraic loop under Backward Euler or Trapezoidal).
- **The bespoke-`step()` blocks** — `vco.rs`, `hysteresis.rs`, `pmsm.rs`. Each module doc
  says, explicitly, why it is **deliberately not a `StateSpace`**:
  - `Vco`: the `[0, 1)` wraparound "is a genuine discontinuity a linear system can't
    express, the same reason `math_ops::saturation` is evaluated directly rather than folded
    into one." Composing `freq = f_nom + k*control` is the caller's job (a `gain` or `Sum`
    ahead of it) — the block's only responsibility is the oscillator itself.
  - `Hysteresis`: the on/off memory "is a genuine discrete latch, not a linear dynamic."
  - `Pmsm`: the back-EMF/cross-coupling terms (`omega_e * l_q * iq`, `omega_e * l_d * id`)
    and the electromagnetic torque (`(l_d - l_q) * id * iq`) are *bilinear* — products of two
    of the block's own states — "so this genuinely cannot be expressed as the descriptor
    system `A x + K dx/dt = B u` every other dynamic block in this crate compiles to." RK4
    over its own 4-state nonlinear vector field, with each stage evaluating the same
    `Pmsm::derivative` at its own trial state — so the bilinear coupling is handled inside
    one block rather than split across blocks that would each need the other's current-step
    output (a cross-block algebraic loop). The angle state is kept *unwrapped* through the
    integration and wrapped to `[0, 2*pi)` only by `Pmsm::theta_e_wrapped`, for callers
    feeding it into `park`/`clarke_park`.
- **The stateless math ops** — `math_ops.rs` (`gain`, `sum`, `product`, `saturation`, and the
  PWM primitive `complementary_pwm_with_deadtime` whose doc comment explains the
  rising-edge-only dead-time semantics every gate-driving modulator shares) and
  `waveform_arithmetic.rs` (the `MathFn1`/`MathFn2`/`MathFn3` function library, plus `table`).
  The module doc of `waveform_arithmetic.rs` lists what was deliberately left *out* and why —
  a finite-difference derivative, noise/random generators (stateful, not stateless math),
  complex-data functions (no complex-valued signals exist), Boolean/comparison operators
  (those belong to a plot-expression language, not simulation blocks; `if`/`limit` cover the
  actual control-logic case). The grouping into one dispatch enum per arity is the same
  pattern `CoordinateTransform` uses — see below.
- **`coordinate_transforms.rs`** — the six Clarke/Park transforms (amplitude convention: the
  standard "2/3" non-power-invariant scaling — a balanced three-phase signal of peak
  amplitude `A` transforms to a `d`/`q` pair of magnitude `A`), the `clarke_park`/
  `clarke_park_inv` convenience compositions, and `anglewrap`. See below.
- **`logic.rs`** — `LogicOp`, `FlipFlopKind`, `LatchPriority`, the `as_bool`/`from_bool`/
  `rising_edge` primitives the block graph's logic evaluation uses; the design rationale
  lives in `logic-signals.md`.

## `CoordinateTransform`: a dispatch enum, and `anglewrap`'s address

Why one enum instead of one `BlockKind` variant per transform — the module doc answers with
the grouping rationale: "`CoordinateTransform` groups them the same way `MathFn1`/`MathFn2`
group the single-output functions, so `dae-runtime`'s block graph can dispatch by one enum
instead of one bespoke `BlockKind` variant per transform." Every transform shares one
input/output/error contract (fixed input count, three outputs, the same `outputs=` naming
convention), so one variant with a `kind` field is the right granularity — the exact grouping
rule `waveform_arithmetic.rs`'s enums follow (and which the Component Reference documents as
one grouped entry per enum, not one entry per function).

`anglewrap`, by contrast, lives in `MathFn2` rather than becoming a seventh
`CoordinateTransform` variant. The deciding factor is stated in the `MathFn2` enum's own doc
comment: unlike every `CoordinateTransform`, `anglewrap` is **single-output** — an ordinary
two-argument function like every other `MathFn2`, "just implemented in
`coordinate_transforms` because it's conceptually paired with `park`/`clarke_park`, which
consume its output." It is the angle-tracking half of a synchronous-reference-frame PLL:
`(alpha, beta) -> theta`, the `atan2(beta, alpha)` angle shifted by a full turn when
negative, so `theta` stays in `[0, 2*pi)` and is a monotonically-sensible input to
`park`/`clarke_park` across a full revolution. The PLL closed loop —
`Clarke -> anglewrap -> Park`, feeding a `Pid` on the `q` output back into the tracked angle
— is spelled out in `book/user-guide/src/coordinate-transforms.md`.

The design reasoning for each of these as it was recorded at build time:
`docs/journal/2026-08.md`, 2026-08-20 22:27 (CoordinateTransform), 2026-08-20 22:39 / 22:52
(Pmsm block, `anglewrap` wired into `MathFn2`), 2026-08-27 12:44 (vector-signal support).

## Source material this was adapted from

- `crates/continuous-blocks/src/lib.rs` — module doc.
- `crates/continuous-blocks/src/{state_space,transfer_function,pid,discrete_pid,vco,hysteresis,pmsm,math_ops,waveform_arithmetic,coordinate_transforms,logic}.rs` — module docs.
- `docs/journal/2026-08.md` — the CoordinateTransform / Pmsm / anglewrap entries cited above.
- `book/dev-guide/src/{discrete-time-blocks,logic-signals,vector-signals,block-graph-descriptor}.md` — cross-referenced.
