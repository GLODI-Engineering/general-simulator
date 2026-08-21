# Three-phase PFC active front end

*(Skeleton — outline below; not yet written.)*

## What goes here
- Stage 1 (ideal): the Clarke->angle_wrapped->Park sync chain and a decoupled dq current loop,
  validated against hand-derived results, no power circuit — a good "here's how you'd unit-test
  a control chain before building the power stage" pattern worth teaching explicitly.
- Stage 2 (real switching bridge): topology, `gate=dutyctrl`/`dutyctrlcomplement` per leg, the
  dynamic PID anti-windup clamp and why it's needed here specifically (a pole-voltage command
  bounded by a still-rising DC bus).
- **Honesty about current status**: as of this writing Stage 2's closed loop does not yet
  converge (q-axis current diverges) — say so plainly with a link to the experiment's own
  Caveats, and update this chapter once that's resolved. Don't present a non-working result as
  working.

## Source material to adapt from
- `internal-archive/experiments/elspice-pwl-pfc-three-phase-vsc/README.md` — this
  chapter is largely a trimmed, user-facing version of that README (drop the "bugs found and
  fixed" narrative detail, which belongs in the dev guide/journal, keep the working recipe and
  the honest status).
