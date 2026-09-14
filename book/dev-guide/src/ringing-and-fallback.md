# Ringing detection and the backward-Euler fallback

## The pathology: A-stable is not L-stable

Trapezoidal integration is A-stable — bounded for any linear, stable mode regardless of step
size — but it is not **L-stable**. A-stability only guarantees a stable mode's numerical
amplification factor stays inside the unit circle; it says nothing about *where* inside the unit
circle. For a very lightly damped mode, trapezoidal's own amplification factor can sit right at
the boundary, near $-1$, which means the mode doesn't decay under trapezoidal integration — it
oscillates at the Nyquist frequency, flipping sign every single step, at roughly constant
amplitude, indefinitely. Backward Euler doesn't have this weakness (it's both A-stable and
L-stable — its amplification factor goes to zero for a stiff mode, not just stays bounded), which
is why it's the fallback rather than some third scheme.

## Where this was actually found

This isn't a theoretical concern kept in the codebase defensively — it's root-caused against a
real, reproducible failure. `crates/dae-runtime/examples/llc_validation.rs` drives an LLC
converter through its ideal-switch dead-time window (both switches gated off, so the switching
node is left nearly floating, with only tiny leakage conductance holding it — exactly the "very
lightly damped mode" case above). During that window, the switch node's voltage oscillated
between roughly $\pm3000\text{V}$ for the entire $\sim$200ns dead-time interval, every single
switching period, while every *other* tracked quantity in the same run — notably the circuit's
actual output — stayed smooth and physically reasonable throughout
(`crates/dae-runtime/src/lib.rs:571-586`, `is_ringing`'s own doc comment records this account
directly). That combination — one specific unknown blowing up while everything electrically
downstream of it stays sane — is the concrete signature that led to writing a detector for this
rather than assuming it away.

## The detection heuristic

`is_ringing` (`crates/dae-runtime/src/lib.rs:587-600`) looks at three consecutive samples of the
same unknown — `x_prev_prev`, `x_prev`, and the current trapezoidal trial — and flags ringing
when all of the following hold simultaneously:

```rust
let alternating = (a > 0.0) == (c > 0.0) && (a > 0.0) != (b > 0.0);
alternating && c.abs() >= b.abs() * 0.9
```

- **The outer two samples share a sign, and the middle one is opposite** (`a` and `c` agree,
  `b` disagrees) — the direct signature of sign-flipping every step, not just *a* sign change.
- **Magnitude isn't shrinking** (`c.abs() >= b.abs() * 0.9`, a 10% tolerance rather than a strict
  inequality, to avoid rejecting a borderline case on floating-point noise alone). This is the
  condition that actually distinguishes sustained ringing from a legitimate settling transient: a
  real, physically decaying oscillation also alternates sign sample to sample near a zero
  crossing, but each successive amplitude is *smaller* than the last. Ringing's defining
  property is that it *isn't* decaying — magnitude staying roughly flat (or growing) across three
  samples is the entire pathology, so this check is what separates "detected instability" from
  "ordinary settling behavior," which must never trigger a fallback.
- **A noise floor excludes near-zero values entirely** (`NOISE_FLOOR = 1e-9`): if any of the
  three samples is smaller in magnitude than the floor, the check returns `false` immediately.
  Without this, an unknown genuinely settling toward zero would show apparent "sign alternation"
  purely from floating-point noise around zero, and get flagged as ringing when nothing is
  actually wrong.

Each condition in the conjunction exists to rule out one specific false positive — dropping any
one of the three would either miss real ringing or misfire on ordinary transient behavior that
happens to cross zero.

## The full per-step decision policy

Two genuinely different classes of trigger drive the fallback, one knowable *before* solving a
step and one only knowable *after* (`step_with_fallback`, `crates/dae-runtime/src/lib.rs:626-682`;
mirrored in the adaptive path by `step_control::lte_attempt`, `crates/dae-runtime/src/
step_control.rs:118-219`):

**Forced up front** (`force_backward_euler`, decided before any solve is attempted):
the first step of a run, a gate/segment change already known from the previous step's own
bookkeeping (in practice: a resolved `GateBinding` state differing from last step), or an active
ringing cooldown (below). See [dae-integration.md](dae-integration.md) for why the first two of
these specifically require backward Euler's tolerance for an inconsistent starting state.

**Detected after the fact** (only checkable once a trapezoidal trial has actually been computed):
a diode's resolved segment differing from the previous step's own (`classify_segments`,
`lib.rs:555-569`), or `is_ringing` firing against the trial's own result. Either one discards the
just-computed trapezoidal trial and redoes the *same* step with backward Euler instead
(`lib.rs:668-681`) — not a partial correction, a full re-solve.

## The cooldown: why one corrective step isn't enough

`RINGING_COOLDOWN_STEPS = 3` (`crates/dae-runtime/src/lib.rs:610`). Any unforced fallback to
backward Euler — segment change or detected ringing, not the already-forced first-step/gate-change
cases — sets `ringing_cooldown = RINGING_COOLDOWN_STEPS`
(`crates/dae-runtime/src/block_graph.rs:2439-2440`, `:2533-2534`), and every step while
`ringing_cooldown > 0` is itself forced to backward Euler, decrementing the counter by one each
step until it reaches zero and trapezoidal resumes being tried.

The reasoning is stated directly on `RINGING_COOLDOWN_STEPS`'s own doc comment
(`lib.rs:602-609`): a single corrective backward-Euler step pulls the ringing mode's *value* back
to something numerically reasonable, but doesn't fully re-establish "consistent" in the sense
trapezoidal's own derivation needs (see [dae-integration.md](dae-integration.md)'s "consistent
initial conditions" discussion). This was an empirical finding, not an assumption: resuming
trapezoidal immediately after just one corrective step was observed to let a smaller residual
oscillation resume and drift toward a stale-but-stable *wrong* plateau, rather than actually
recovering — worse than simply staying on backward Euler a few steps longer, since a wrong-but-
stable answer is harder to notice than an obviously oscillating one. A short run of
unconditionally-stable backward-Euler steps lets the correction actually settle before trusting
the second-order scheme's stricter assumptions again.

The same cooldown mechanism, and the same reasoning, governs the adaptive-step-size loop's own
next-`dt` suggestion: after any forced or discontinuity step, `suggested_dt_next` is conservatively
halved rather than left to the ordinary local-truncation-error growth formula
(`step_control.rs:110-116`, `:144`, `:184`) — trust the solution *less* immediately after a mode
change, not more, exactly mirroring why the cooldown exists at all.

## A methodology note for diagnosing a future instability like this

The general pattern worth keeping, beyond this one specific fix: when a transient result shows
one isolated unknown behaving pathologically while everything electrically connected to it stays
smooth, suspect the *integration scheme's* stability properties at that specific operating
condition before suspecting the device models or the LCP fold. The LLC dead-time case had every
hallmark of a modeling bug at first glance (a switch node at $\pm3000\text{V}$ looks like a sign
error or a wrong node reference) — the actual cause was a numerical-stability property of
trapezoidal integration under a specific, lightly-damped circuit condition (a floating node
during dead time), not the circuit model at all. The diagnostic signal that pointed the right
way was exactly the *combination* checked by the three conditions in `is_ringing`: strict
sign-alternation, non-decaying magnitude, well above the noise floor, isolated to one region of
the run (the dead-time windows specifically, not the whole trace). A future instability that
shares that signature — sign-flipping every step, roughly constant amplitude, appearing right
after a mode change — is very likely the same class of problem, and the same fix (detect it,
fall back to the unconditionally-stable scheme, hold the fallback for a few steps) is the right
first thing to try before assuming a new mechanism is needed.
