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

### What this heuristic cannot see

The test is on the *sign* of the raw unknown, so it only detects an oscillation that straddles
zero. A switching node ringing about a $400\text{V}$ or $800\text{V}$ rail — the ordinary case
in a converter, as opposed to the floating dead-time node this detector was written for — never
changes sign, and `is_ringing` is blind to it however badly it oscillates. That blindness is a
known, deliberate limitation rather than an oversight to be fixed in passing: the offset-invariant
signature is sign alternation of the *increments* $x_{n} - x_{n-1}$, and three samples cannot tell
that apart from one ordinary turning point of a smooth waveform, so a detector built on it needs
a fourth sample of history and a discriminator that is not simply a magnitude ratio. It is
tracked as issue #116.

What makes the blindness survivable is the policy below: the cooldown keeps the solver on
backward Euler long enough after each fresh excitation that the mode is settled before
trapezoidal is trusted again, so the detector is a backstop rather than the primary defence.

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

`RINGING_COOLDOWN_STEPS = 3`. Every backward-Euler step taken for a *fresh* reason — the first
step of a run, a gate state that just changed, a diode segment change, or detected ringing —
sets `ringing_cooldown = RINGING_COOLDOWN_STEPS`, and every step while `ringing_cooldown > 0` is
itself forced to backward Euler, decrementing the counter by one each step until it reaches zero
and trapezoidal resumes being tried. The one case that must *not* restart the counter is a
backward-Euler step the cooldown itself forced, or the cooldown could never end; all five
transient loops in `dae-runtime` get that distinction from the single function
`next_ringing_cooldown`, which exists because when each loop spelled the policy out for itself,
all five spelled it out wrong in the same way.

### The wrong way this was spelled out, and what it cost

Until the fix, the condition was "an *unforced* fallback restarts the cooldown" — which excluded
exactly the two reasons already known before solving, the first step of a run and a gate change.
A gate transition therefore got precisely one backward-Euler step, and trapezoidal resumed
immediately on a switching node whose stiff mode was not settled.

That mode is the one formed by a conducting switch's resistance and the capacitance on its node,
$\tau = R_{on} C_{node}$, which is picoseconds to a nanosecond in a real converter — orders of
magnitude below any transient step size. Backward Euler's amplification factor for it,
$1 / (1 + \Delta t / \tau)$, removes essentially all of it in one step when
$\Delta t \gg \tau$, which is why a single step was enough to hide the defect at coarse steps.
Trapezoidal's, $(1 - \Delta t / 2\tau) / (1 + \Delta t / 2\tau)$, sits just inside $-1$ in the
same regime: whatever the one corrective step left behind then flipped sign every step and barely
decayed, about the node's own rail, where the sign test in `is_ringing` could not see it.

The result was a switching node sitting tens of volts outside the window its own body diodes
clamp it to — a voltage-stress figure read off such a run was simply wrong — and *worse* at a
smaller step, because the smaller the step the less of the stiff mode that one corrective step
removed. `crates/dae-runtime/tests/body_diode_clamp.rs` pins it with a three-leg bridge whose
legs carry $174\text{pF}$, $1.65\text{nF}$ and $16.5\text{nF}$ across each switch, so that the
three $\tau$ values are three decades apart and *some* leg is mishandled at every step size
tested. Before the fix its nodes reached $427.9\text{V}$, $428.1\text{V}$ and $428.1\text{V}$ at
$5\text{ns}$, $0.5\text{ns}$ and $0.05\text{ns}$ against a $400\text{V}$ rail whose clamp is
$401\text{V}$ — a different leg each time — and dipped to $-25.9\text{V}$ against a $-1.0\text{V}$
clamp. With the cooldown applied after a gate change they stay within $400.2\text{V}$ to
$400.6\text{V}$, and the same node's peak varies by under $0.1\text{V}$ across all three step
sizes.

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
