# Troubleshooting and gotchas

*(Skeleton — outline below; not yet written.)*

## What goes here
- One entry per real, reproducible trap — same discipline as `docs/gotchas/`: symptom, cause,
  fix. Candidates already known:
  - MOSFET node order is `(drain, source)`, not `(source, drain)`.
  - Every MOSFET in one run must share `r_on`.
  - `--mode dc` rejects block-driven gates.
  - Adaptive stepping needs `cscript_clone` on every `CScript` block.
  - A fixed PID anti-windup clamp sized for a final steady-state value can silently fail to
    engage during a startup transient where the true achievable range is still much smaller —
    use `clamp_lo_in=`/`clamp_hi_in=` instead when the bound is itself state-dependent.
  - An `UnknownBlockInput` error means a name doesn't exist anywhere in the graph, never a
    same-step-ordering issue (order is derived automatically); `AlgebraicLoop` is the error for
    a genuine cycle, with the exact closing path.
- Keep each entry short and searchable (readers will `Ctrl-F` an error message here).

## Source material to adapt from
- `docs/gotchas/` (the internal, contributor-facing version — this chapter is the
  user-facing subset, filtered to things a *user* of the CLI/netlist grammar would hit, not
  internal numerical/pivoting traps a contributor to the solver itself would hit — those belong
  in the dev guide instead).
