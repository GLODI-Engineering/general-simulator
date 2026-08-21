# Verification discipline

*(Skeleton — outline below; not yet written.)*

## What goes here
- The core rule stated first and plainly: numerical agreement alone is not proof. For any new
  device model, LCP formulation, or continuous block: derive the expected result
  independently, and test *that* — not just internal self-consistency.
- A worked taxonomy of "what counts as independent," with a real example of each already in
  this codebase:
  - Hand-derived closed-form (e.g. the decoupled R-L circuit exact solution for `Pmsm`'s
    `d`-axis test).
  - Convergence-order checks against a converged reference (e.g. `Pmsm`'s RK4 4th-order test) —
    and why this is *not* the same as "smaller error at smaller dt," and why that distinction
    matters.
  - Cross-checking a fused/convenience implementation against composing its own primitives
    (e.g. `clarke_park` vs. `clarke` then `park`) — and why this caught a real bug class (the
    upstream d/q-swap the source material's own changelog records).
  - Cross-checking against a previously-validated Xyce/ngspice baseline in the sibling
    `internal-archive` repo, where a matching topology exists.
- What this discipline is *for*: cite the boost-PI anti-windup incident
  (`gotchas/xyce-boost-pi-nonlinear-failure-integrator-windup.md`) as the concrete cautionary
  tale — a plausible-looking final number that was actually just a capacitor discharging, not
  real regulation.

## Source material to adapt from
- `AGENTS.md`'s "Verification discipline" section — port directly, then expand with the
  worked-example taxonomy pulling from this session's own additions (`Pmsm`, `CoordinateTransform`,
  `topological_order`) as fresh, still-recent case studies.
