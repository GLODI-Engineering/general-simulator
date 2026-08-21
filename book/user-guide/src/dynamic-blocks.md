# Dynamic blocks (PID, state-space, transfer function)

*(Skeleton — outline below; not yet written.)*

## What goes here
- `kind=pid`: fields, the anti-windup clamp, and **both** clamp forms — `clamp_lo=`/
  `clamp_hi=` (fixed) and `clamp_lo_in=`/`clamp_hi_in=` (dynamic, read from the graph every
  step) — with the worked "why you'd want dynamic" example (a current loop whose achievable
  output range depends on a still-rising DC bus).
- `kind=statespace` and `kind=tf`: field reference, and when to reach for one over the other
  (arbitrary `(A,B,C,D)` vs. a rational `N(s)/D(s)`, e.g. a PID's own filtered-derivative form).
- `kind=vco` and `kind=hysteresis`: what makes each "not a state-space" (a real discontinuity —
  wraparound, or a latch — that a linear system can't express), briefly; full rationale belongs
  in the dev guide.
- A short "how do I close a loop through one of these" example reusing `prev:`/`meas:` from the
  signals chapter.

## Source material to adapt from
- `crates/dae-runtime/src/block_graph.rs`'s `BlockKind` doc comments for `Pid`/`PidClamp`/
  `StateSpace`/`TransferFunction`/`Vco`/`Hysteresis`.
- The PFC/PMSM worked examples' own PI-gain derivations (IMC pole-cancellation) are a good
  source for the "how do I actually pick Kp/Ki" worked-example content this chapter should have.
