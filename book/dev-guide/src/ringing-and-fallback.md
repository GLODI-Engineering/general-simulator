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

`is_ringing` looks at the four most recent consecutive samples of the same unknown — the three
most recent accepted states and the current trapezoidal trial — and asks
`unknown_is_ringing` about each unknown in turn. The signature is on the *increments*

$$d_1 = a - w,\qquad d_2 = b - a,\qquad d_3 = c - b$$

of the four samples $w, a, b, c$, not on the samples themselves, which is what makes it
independent of whatever DC offset the unknown sits on. Ringing is flagged when all three of
the following hold at once:

- **The increments alternate in sign twice in a row** — $d_1$ against $d_2$, and $d_2$ against
  $d_3$. This is the direct signature of a mode that reverses direction on *every* step, and
  it is the condition that needs the fourth sample. A Nyquist mode alternates for as many
  increments as one cares to look at; an ordinary turning point of a smooth waveform alternates
  once and then does not.
- **The increments are not shrinking** ($|d_2| \ge 0.9|d_1|$ and $|d_3| \ge 0.9|d_2|$, a 10%
  tolerance rather than a strict inequality so floating-point noise alone does not reject a
  borderline case). This is what separates sustained ringing from a physically decaying
  oscillation, which also reverses direction every step as it dies but with a shrinking
  amplitude. Not decaying is the entire pathology.
- **The oscillation is large relative to the unknown's own scale**
  ($|d_2| \ge 10^{-6}\max(|w|,|a|,|b|,|c|)$). An offset-free signature would otherwise fire on
  the last bits of the solve wobbling under a settled $400\text{V}$ rail, whose increments do
  alternate and do not decay. $10^{-6}$ sits a long way above a linear solve's own relative
  error (nearer $10^{-12}$) and a long way below any oscillation worth a fallback.

Both thresholds are dimensionless ratios. That is deliberate: the previous version of this
test used an absolute `NOISE_FLOOR = 1e-9` on the raw sample, applied to a vector whose rows
are volts in one place and amps in another, so it discarded a current legitimately living at
$10^{-10}\text{A}$ for no reason other than its unit — while doing nothing about round-off on a
rail. Neither of the two ratios depends on the unknown's unit or on $\Delta t$.

### Why three samples were not enough

Until #116 the test was on the *sign of the raw sample*: the outer two of three consecutive
samples sharing a sign with the middle one opposing it.

```rust
let alternating = (a > 0.0) == (c > 0.0) && (a > 0.0) != (b > 0.0);
alternating && c.abs() >= b.abs() * 0.9
```

That only ever saw an oscillation that straddled zero. A switching node ringing
$\pm50\text{V}$ about a $400\text{V}$ rail gives three positive samples — 350, 429, 386 — and
was invisible to it however badly it oscillated. In a converter most nodes sit on or near a
rail, so the floating dead-time node this detector was first written against was the special
case, not the general one; the blindness was found in #113, where a switching node rang about
its $400\text{V}$ rail tens of volts past the clamp its body diode holds it to and the
detector never fired.

The obvious repair — the same two conditions, but on $d_1$ and $d_2$ instead of on the signs —
does not work, and it is worth recording *why*, because it looks like it should. At a smooth
maximum the two increments are also opposed in sign and are also nearly equal in magnitude
(exactly equal, for samples placed symmetrically about the peak), so no magnitude ratio
separates the two cases. A three-sample test on increments therefore falls back at every
extremum of every oscillating quantity. `tests/ringing_about_a_rail.rs` measures what that
costs on a lossless $LC$ tank ringing about a $400\text{V}$ rail at $\Delta t = T/33$: 69% of
the waveform's amplitude gone over five cycles, and a measured convergence ratio of 1.0
instead of 4.0 — the whole run reduced below first order. Those two numbers are what the
fourth sample buys.

The cost is that the loops and `checkpoint::Checkpoint` carry a third history vector,
`x_prev3`. A resumed run has to have it, or its first steps would decide differently from the
uninterrupted run's and break the bit-identity contract, so `FORMAT_VERSION` went to 2 and a
checkpoint written by an older build is refused up front by name rather than misread.

The cooldown below is still what keeps the detector a backstop rather than the primary
defence: it holds the solver on backward Euler long enough after each fresh excitation that
the stiff mode is settled before trapezoidal is trusted again.

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
