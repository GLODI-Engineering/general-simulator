# Fixed vs. adaptive time stepping

*(Skeleton — outline below; not yet written.)*

## What goes here
- `--dt <fixed>`: deterministic, exactly reproducible, the right default while developing a
  new netlist (easiest to debug against).
- Adaptive (`--dt` omitted): local-truncation-error-controlled, small steps where the solution
  changes fast, large where it's settled — the same approach every real SPICE-family tool
  defaults to. When it's worth switching to (long runs, widely varying timescales).
- How to pick `--dt` in the fixed case: resolve the fastest thing in the circuit (switching
  period, current-loop bandwidth) with enough samples — a rule of thumb plus one worked example
  showing an under-resolved run's visible symptom.
- Forward pointer to the dev guide for *why* adaptive stepping needs `cscript_clone` and how
  backward-Euler fallback interacts with it — this chapter stays at the "how do I use it" level.

## Source material to adapt from
- `crates/dae-runtime/src/step_control.rs`'s `TimeStep`/`AdaptiveConfig` doc comments.
- Any worked example's own `--dt` choice-and-rationale comment (several `.cir` files in the
  `internal-archive` experiments explain their own `dt` pick inline).
