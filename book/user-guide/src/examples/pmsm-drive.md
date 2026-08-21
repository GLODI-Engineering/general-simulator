# PMSM field-oriented-control drive

*(Skeleton — outline below; not yet written.)*

## What goes here
- Stage 1 (ideal): id=0 FOC speed control driving the `Pmsm` block directly, both loops closed
  via `prev:` around the motor's own previous-step output — a clean, minimal illustration of
  the `prev:` pattern generally, worth cross-referencing from `signals.md`.
- The cascade gain-derivation walkthrough (current loop, then the reduced first-order
  mechanical plant for the speed loop) as a template readers can reuse for their own motors.
- Stage 2 (real switching inverter): note as not-yet-built if still the case when this chapter
  is actually written; keep this honest the same way the PFC chapter must be.

## Source material to adapt from
- `internal-archive/experiments/elspice-pwl-pmsm-foc-drive/README.md` — trim to the
  user-facing recipe + verified results, same approach as the PFC chapter.
