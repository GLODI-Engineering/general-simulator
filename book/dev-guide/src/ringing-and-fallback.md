# Ringing detection and the backward-Euler fallback

*(Skeleton — outline below; not yet written.)*

## What goes here
- Why trapezoidal (A-stable, not L-stable) can let a lightly-damped mode ring at the Nyquist
  frequency (sign-flipping every step at roughly constant amplitude) instead of decaying — the
  concrete case that found this: a MOSFET dead-time window, switch node oscillating ~+/-3000V
  while every other tracked quantity stayed smooth.
- The exact detection heuristic: three consecutive samples of the same unknown alternating in
  sign, magnitude not shrinking, above a noise floor — and why each condition in that
  conjunction is there (excluding legitimate settling sign changes and near-zero noise).
- The full per-step decision policy: forced-backward-Euler triggers (first step, gate/segment
  change, active cooldown) vs. detected-after-the-fact triggers (segment changed as a *result*
  of the trapezoidal trial, or ringing) — and why a 3-step cooldown follows any unforced
  fallback rather than resuming trapezoidal immediately (one corrective step isn't enough to
  fully re-satisfy "consistent initial conditions").
- This is a good chapter for a "how would I diagnose a new instability like this" methodology
  note, not just this one case — it's a template for future contributors.

## Source material to adapt from
- `crates/dae-runtime/src/lib.rs`: `is_ringing`, `RINGING_COOLDOWN_STEPS`, `step_with_fallback`
  doc comments — the full story, including where/how it was found, is already written there.
- `crates/dae-runtime/examples/llc_validation.rs` as the concrete reproduction case to cite.
