# Fixed vs. adaptive time stepping

`general-simulator` steps a transient run one of two ways: a fixed step size you choose
(`--dt`), or an adaptive, local-truncation-error-controlled step size the solver chooses for
itself (the default whenever `--dt` is omitted). This chapter covers both, the flags that tune
adaptive stepping, `--out-every`'s deliberately separate job, and a real, honest case where
adaptive stepping simply doesn't work well — not a bug, a genuine numerical-stiffness limit
worth knowing about before you hit it on a real circuit.

## `--dt <fixed>`: deterministic, the right default while developing a netlist

`--dt` fixes the step size for the whole run: deterministic, exactly reproducible, and the
easiest to debug against — the same `.cir` file with the same `--dt` always produces exactly the
same output, which matters while you're still finding out whether a new netlist is even correct.
`general-simulator first.cir --mode transient --tfinal 0.5 --dt 0.01` (from [Your first
netlist](first-netlist.md)) is this mode.

Picking `--dt`: resolve the fastest thing in the circuit — a switching period, a current-loop
bandwidth — with enough samples per cycle that the waveform you're looking at is actually
trustworthy, not an aliased or under-resolved approximation of it. An under-resolved fixed `--dt`
doesn't fail loudly: it just quietly misses fast transitions, the same way any fixed-step
simulator would. `--dt` and any `--dt-max`/`--dt-min`/`--dt-init`/`--reltol`/`--abstol` flag are
mutually exclusive — you're choosing one stepping strategy, not blending both.

## Adaptive stepping (omit `--dt`): small steps where it matters, large where it's settled

Omit `--dt` (optionally tuning `--dt-max`/`--dt-min`/`--dt-init`/`--reltol`/`--abstol`) for
adaptive step-size control instead — the same local-truncation-error approach every real
SPICE-family tool defaults to: small steps where the solution is changing fast, large steps
where it's settled. Worth switching to for a long run with widely varying timescales, where a
fixed step fine enough for the fast parts would waste enormous effort resolving the boring, flat
parts of the same run at the same resolution.

The five tuning flags (`crates/dae-runtime/src/step_control.rs`'s `AdaptiveConfig`):

- `--dt-init` — the very first trial step size.
- `--dt-min` — the floor: if the controller would want to shrink below this, it accepts the
  result anyway rather than looping forever chasing a smaller step. A run that only converges
  below `dt-min` needs a smaller `dt-min` or looser tolerances, not an infinite retry.
- `--dt-max` — the ceiling: the largest step the controller will ever propose, however settled
  the solution looks.
- `--reltol` / `--abstol` — together form the per-unknown error scale

  $$\text{abstol} + \text{reltol} \cdot \max(|x_t|, |x_{t+dt}|)$$

  the same `RELTOL`/`ABSTOL` idea every SPICE-family tool exposes.
  `reltol` alone isn't enough for a signal that legitimately crosses zero in its own course (an
  AC-coupled voltage, an inductor current) — `abstol` is the floor that keeps step control
  meaningful there.

Leaving all five unset derives sensible defaults from `--tfinal` alone
(`AdaptiveConfig::from_t_final`):

$$dt_{max} = \frac{t_{final}}{1000}, \quad dt_{init} = \frac{dt_{max}}{100}, \quad
dt_{min} = dt_{max} \times 10^{-9}, \quad \text{reltol} = 10^{-3}, \quad \text{abstol} = 10^{-9}$$

— $dt_{max}$ is roughly the point count a typical fixed SPICE run would use, $dt_{init}$ starts
conservatively since nothing about the circuit's own timescale is known yet. That last default
is exactly what makes the caveat below bite on a large-signal circuit —
`abstol` sized for one circuit's own signal magnitudes doesn't automatically transfer to another.

**`--max-steps N`** caps how many accepted adaptive steps one run may take (default
10,000,000) before aborting with `DaeError::AdaptiveStepStalled` instead of growing its
in-memory row trace without limit — a real safeguard, not a theoretical one: a stalled step
controller (or a pathological netlist/tolerance combination) grew an actual run's memory use
without bound before this flag existed. Only relevant under adaptive stepping — `--dt` runs a
fixed, precomputed number of steps and can't stall this way.

## Adaptive stepping now lands exactly on discrete/PWM edges

A real, recently-fixed bug worth knowing existed even though it's fixed: `TimeStep::Adaptive`
could previously miss a discrete or PWM gate edge outright once `dt_max` approached or exceeded
a switching period — the step controller had no notion that anything *discrete* was about to
happen partway through a large candidate step, so it could step clean over a transition. The fix,
`earliest_next_event_dt` (`crates/dae-runtime/src/block_graph.rs`), computes the earliest
relative `dt` at which a `kind=pwm`/`kind=pspwm` gate edge or a fixed-sample-rate block's next
sample hit would occur, and clamps the adaptive controller's candidate step to land exactly on
it instead — a real, previously-open correctness gap, not a performance tweak. `kind=vco`
(a continuous ramp, no discrete edge to land on) is deliberately excluded from this clamp, since
there's nothing discrete about its own output to miss.

## `ic=` and how a transient run starts

An `ic=`-declared capacitor voltage or inductor current changes what the very first step of a
transient run starts from, not just the netlist's own bookkeeping: see [Grammar
overview](netlist-grammar.md#ic-initial-conditions) for the full sign-convention and
assignment-semantics detail. The short version relevant here: the declared states are written
directly into the starting state vector and everything else starts at rest — the operating point
is skipped, not solved — and the circuit's own algebraic constraints (which a solved operating
point would already satisfy) are only re-imposed once the first backward-Euler step runs. This
applies identically whether that first step is fixed-size or the first adaptive trial.

## `--out-every N`: thins what's written, never what's computed

`--out-every N` writes only every Nth resolved point (default 1, every point). This is a
*completely different knob* from `--dt`/the adaptive settings, and conflating the two is a
common point of confusion:

- `--dt`/`--dt-max`/etc. control what gets **computed** — the actual integration step, which
  determines the numerical accuracy of the solution.
- `--out-every` controls what gets **written** — every step is still taken, the solution is
  unchanged, and `kind=measure` still sees every point; only serialization to the CSV/rawfile is
  skipped for the rows in between.

This exists because the timestep a circuit genuinely needs and the output rate a human or a
downstream tool needs can legitimately differ by orders of magnitude: verifying zero-voltage
switching needs a step below $r_{on} \cdot C_{oss}$ (0.4 ps at 1 mΩ and 400 pF), which is 3.75
million
steps across a 150 ns commutation window — while the measurement itself only needs a few
thousand points. Without `--out-every`, that run is affordable to *compute* and not to *write
down*. `kind=measure` results (see [Post-processing measurements](measurements.md)) are computed
from the full, undecimated trace regardless of `--out-every`.

## `kind=octblock` cannot run under adaptive stepping — permanently

Unlike `kind=cscript` (where `cscript_clone`'s absence is a fixable opt-in gap — see [The
CScript escape hatch](cscript.md)), `kind=octblock`'s incompatibility with adaptive stepping is
architectural and permanent: `DaeError::OctBlockDoesNotSupportAdaptiveStep`. Octave-side state
for an `octblock` instance lives in one shared `octave-cli` session, which is never cloned — a
rejected adaptive trial's own mutating calls into that session would otherwise silently persist
with no way to roll them back, unlike every other block kind's own state. There's no `.m`-file
convention that could make Octave's own shared global-struct state trial-cloneable the way
`cscript_clone` makes a C state pointer cloneable. Any netlist using `kind=octblock` needs a
fixed `--dt`; this isn't a bug to watch for a fix on.

## A real caveat: adaptive stepping can still be numerically impractical

Adaptive stepping is not a universal win, even after the edge-landing fix above. A real
investigation against a six-leg, 300-600 kHz phase-shift-PWM converter (an internal
phase-shift-PWM modulator comparison experiment, "Part 2: why
`TimeStep::Adaptive` doesn't work here") found adaptive stepping numerically impractical for that
circuit, honestly documented rather than papered over:

- A first attempt (`--dt-max 5e-6 --abstol 1e-9`, tolerances copied from an earlier,
  smaller-amplitude synthetic test without rescaling) drove the machine to roughly 11 GB RSS / 24
  GB swap in under 90 seconds and had to be killed — a 200 µs diagnostic window showed a median
  step size around 1.6 ns, over 2500× smaller than the requested `dt_max`, because the LTE
  acceptance criterion ($\text{abstol} + \text{reltol} \cdot |x|$) was dominated by an `abstol`
  far too tight for this circuit's actual $O(10\text{-}100\,\text{V})/O(1\text{-}5\,\text{A})$
  signal magnitudes.
- A second attempt with `abstol` rescaled to the circuit's real magnitudes improved a 200 µs
  diagnostic window from 104K to 34K steps — genuine, confirmed improvement — but the full
  20 ms run still had to be killed after 18 minutes at 10.9 GB RSS with 24 GB of swap in use,
  well past what the diagnostic window's own linear extrapolation predicted: this circuit's
  stiffness (six independently phase-shift-modulated legs, each with its own soft-switching
  zero-current/zero-voltage crossings) evidently gets substantially worse later in the AC cycle
  than the opening window suggested.

The honest conclusion from that investigation: `TimeStep::Adaptive`, even with the event-landing
fix, was not practical for that specific circuit at the full 20 ms scale on the machine
available — a second, independent numerical-stiffness problem, not the missed-edge bug the fix
targeted. Fixed stepping, with a `--dt` chosen conservatively relative to the switching
frequency, was the practical choice there instead. Two general lessons worth carrying forward:
`abstol`'s default (or a value copied from a different circuit) needs rescaling to your own
circuit's actual signal magnitudes before adaptive stepping is a fair test at all, and a
sufficiently stiff, frequently-switching circuit may simply cost adaptive stepping more than it
saves — try it, watch memory, and fall back to fixed stepping without a struggle if it's not
converging cheaply.

Fixed stepping isn't automatically safe either, to be clear: the same source notes that a `--dt`
fine enough to never miss an edge is a choice the user makes by hand, with no tool-side check —
exactly the failure mode the adaptive-side `earliest_next_event_dt` fix above exists to close on
the adaptive side. Neither mode is a free pass against an under-resolved step.
