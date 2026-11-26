---
id: GOTCHA-005
discovered: 2026-11-26
discovered_by: contributor
scope:
  - crates/dae-runtime/src/lib.rs
severity: high
status: fixed
reproducibility: full
tags: [integration, trapezoidal, backward-euler, ringing, convergence]
---

# GOTCHA-005 — Three samples cannot tell a Nyquist oscillation from a smooth turning point

## Symptom

A ringing detector written on three consecutive samples is either blind to half the cases it
exists for, or it fires on waveforms that are perfectly healthy — and there is no threshold
that gets both. Taken the second way, the run keeps producing plausible-looking numbers while
silently losing most of its amplitude and all of its second order.

## Reproduction

A lossless $LC$ tank ringing about a rail, which never crosses zero
(`crates/dae-runtime/tests/ringing_about_a_rail.rs`):

```
V1 a 0 400
L1 a b 1e-3
C1 b 0 1e-6 ic=600
```

$\omega = 1/\sqrt{LC} = 31622.78\ \text{rad/s}$, $T = 198.692\mu\text{s}$, exact solution
$v_C(t) = 400 + 200\cos(\omega t)$ with a $200\text{V}$ envelope that never decays. Run it at
a deliberately coarse $\Delta t = 6\mu\text{s} \approx T/33$ with `is_ringing` written on the
increments of **three** samples — alternating sign, magnitude not shrinking:

```rust
let (d1, d2) = (b - a, c - b);
let alternating = (d1 > 0.0) != (d2 > 0.0);
alternating && d2.abs() >= d1.abs() * 0.9
```

Peak-to-peak of `V(b)` goes from $336.3\text{V}$ in the first cycle to $102.8\text{V}$ in the
sixth — **69.4% of the amplitude gone** — and the error ratio between $\Delta t$ and
$\Delta t/2$ measures **1.0** where second order is 4.0 and first order is 2.0. Nothing in the
output says a fallback happened.

## Root cause

At a smooth maximum the two increments either side of the peak are opposed in sign *and*
equal in magnitude — exactly equal, for samples placed symmetrically about it. That is the
same signature a Nyquist mode shows, so no magnitude ratio separates the two, and the
detector fires at every extremum of every oscillating quantity. Each firing costs one
backward-Euler step plus `RINGING_COOLDOWN_STEPS` more, which at $\omega\Delta t = 0.19$
damps the mode by roughly $(\omega\Delta t)^2/2 \approx 1.8\%$ per step.

The three-sample test it replaced was on the raw samples' *signs* instead, which fails the
other way: offset-blind, so an oscillation about a $400\text{V}$ rail is invisible (#116, and
#113 in the field).

## Fix or workaround

Carry a fourth sample and require **two consecutive** sign alternations of the increments. A
Nyquist mode reverses direction on every step and alternates for as many increments as one
cares to look at; a smooth turning point reverses once and then does not. The cost is a third
history vector (`x_prev3`) through all five transient loops and into
`checkpoint::Checkpoint`, which bumped `FORMAT_VERSION` to 2.

Also replace the absolute noise floor with one relative to the unknown's own scale: an
offset-free signature fires on round-off wobble under a settled rail, and an absolute floor
applied to a vector whose rows are volts in one place and amps in another excludes a small
current for no reason but its unit.

## Prevention

`crates/dae-runtime/tests/ringing_about_a_rail.rs` holds this direction shut with two
measurements rather than an inspection — amplitude preserved across five cycles, and a
convergence ratio near 4.0 through the real transient loop. Both fail loudly on the
three-sample version, which is how they were checked to have teeth before the fix went in.
`ringing_detector_tests` in `lib.rs` holds the other direction.

## References

- `GLODI-Engineering/general-simulator#116` — the issue.
- `GLODI-Engineering/general-simulator#113` and
  [GOTCHA-004](GOTCHA-004-one-backward-euler-step-after-a-gate-change.md) — where the
  blindness was first met.
- `book/dev-guide/src/ringing-and-fallback.md` — the full policy.

## History

- 2026-11-26: Discovered while fixing #116, and fixed in the same change.
