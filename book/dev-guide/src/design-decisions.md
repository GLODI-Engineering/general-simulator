# Design decisions log

*(Skeleton — outline below; not yet written.)*

## What goes here
- A running, dated list of "we considered X, chose Y, because Z" entries — shorter and more
  scannable than the journal (which is chronological narrative), organized by topic instead of
  by date. Candidates already well-documented enough to write up directly:
  - PWL segments via LCP vs. Newton + voltage limiting.
  - Ideal-switch channel state: exogenous command vs. LCP-resolved (`docs/journal/2026-08.md`,
    "Robustness Q&A...", Q5).
  - Block-graph execution order: derived topological sort vs. manual declaration order
    (same journal entry, Q1/Q2).
  - `Signal::BlockPrev` vs. an alternative that would have let same-step cycles exist with a
    special resolution rule (why the one-sample-delay design was preferred).
  - Per-block RK4 vs. one fused ODE across the whole block graph (once that question's answer
    exists — see `block-graph-rk4.md`).
  - Bench-scale vs. mains-scale for the PFC switching demo, and *not* forcing a "unify
    ideal-switch switching" fix under time pressure — decisions made explicitly, not silently,
    worth recording as decisions even when the underlying problem stayed open.
  - **Enforced physical/signal-domain converters, implemented 2026-08-21** (`BlockKind::Phys2Sig`/
    `Sig2Gate`/`Sig2Voltage`/`Sig2Current`, `docs/journal/2026-08.md` for the full account) —
    no longer an open question, this entry is now `## Physical/signal-domain converters` proper
    (not just a pointer), since the decision and its rationale are settled: user-requested,
    modeled directly on a real block-diagram tool's own physical/signal converter split,
    enforced at the netlist/`dae-runtime` level (not just a UI convention) so a violation is a build-time
    error (`DaeError::GateTargetNotSig2Gate`/`SourceNotSig2PhysicalConverter`) regardless of
    which tool produced the netlist. **Revised 2026-08-27**: `Sig2Gate` was removed and merged
    into `Sig2Voltage` (an ideal switch's gate is itself a voltage, not a distinct discrete-actuation
    signal domain — no dedicated gate-only converter needed); `DaeError::GateTargetNotSig2Gate`
    was renamed `GateTargetNotSig2Voltage` to match. Two converters remain (`Sig2Voltage`/
    `Sig2Current`), not three. **Revised 2026-09-01**: `Sig2Voltage`/`Sig2Current` were
    themselves merged into one `BlockKind::Sig2Phys { domain: PhysicalDomain }` variant
    (`domain=voltage`/`domain=current`) — same rationale as the `Sig2Gate` merger above: the two
    were identical in shape and evaluation, differing only in which device letter/`GateBinding`
    target they're allowed to satisfy, which is now expressed as a `PhysicalDomain` field instead
    of a second enum variant. `DaeError::GateTargetNotSig2Voltage`/
    `SourceNotSig2PhysicalConverter` keep their names (the enforcement semantics — gate must be
    `domain=voltage`, `V`/`I` sources must match their own device letter's domain — are
    unchanged); only the netlist syntax (`kind=sig2phys domain=<voltage|current> in=<signal>`)
    and the underlying Rust type changed. One converter type remains, parameterized by domain.
    **Revised 2026-09-01**: `kind=probe` (`BlockKind::Probe`/`ProbeTarget`) was renamed
    `kind=phys2sig` (`BlockKind::Phys2Sig`/`Phys2SigTarget`), for naming symmetry with
    `sig2phys` — `phys2sig` reads a circuit quantity into the signal domain, `sig2phys` drives a
    signal-domain value onto a circuit quantity. Pure rename: fields, validation (including the
    `branch=` no-current-unknown build-time error and its ammeter-idiom message), and behavior
    are unchanged; no back-compat alias for the old `probe` name.
  - **Signal-domain `pwc`/`pwl` source naming, resolved 2026-08-23** (`docs/journal/2026-08.md`,
    `2026-08-23 15:24`/`15:52`) — no longer an open question, this entry is now
    `## Signal-domain source naming: pwc vs. pwl` proper (not just a pointer): adding
    `SIN`/`PULSE`/`EXP`/`PWL`/`SFFM` to the signal domain (parity with `general-mna`'s own
    electrical-domain source forms) collided with the *existing* `BlockKind::Pwl`, which was
    piecewise-*constant* (a step-schedule block, not the piecewise-*linear* interpolation the
    name implies). User's own proposed resolution: rename the old block `Pwc`, freeing `Pwl` for
    real SPICE-matching piecewise-linear semantics — settled, implemented, every prior use
    mechanically renamed and re-verified byte-identical.
- Format: one entry per decision, `## <short title>`, `**Considered:**`, `**Chose:**`,
  `**Because:**`, `**Revisit if:**` (conditions that would reopen the question).

## Signal-domain source naming: pwc vs. pwl

**Considered:**
1. Keep `kind=pwl` meaning what it already meant (piecewise-constant, used for step reference
   schedules), and give the new SPICE-matching piecewise-linear source a different name (e.g.
   `pwl_lin`, `spice_pwl`).
2. Change `kind=pwl`'s own interpolation to real piecewise-linear, matching the electrical
   domain's `PWL(...)` source exactly, letting every existing user of the name adopt the new
   (correct-per-the-name) behavior automatically.
3. Rename the existing piecewise-constant block to `pwc`, freeing `pwl` for the new
   piecewise-linear source.

**Chose:** Option 3 (user's own proposal).

**Because:** Option 1 keeps two similarly-named blocks (`pwl`/`pwl_lin`) with *different*
interpolation behavior, exactly the kind of "easy to mix up because they sound like the same
thing" trap `general-mna`'s own `PwlPoints` doc comment already warns readers about for the
unrelated block-graph/electrical-domain naming overlap. Option 2 is a silent breaking change:
every existing reference-schedule netlist in this project's own `internal-archive`
sibling repo (e.g. a speed-step schedule `REF kind=pwl points=[[0,800],[0.05,1500]]`) relies on the
*step* behavior — reinterpreting it as a ramp would silently corrupt every one of those
experiments' own recorded results without a single line of `general-simulator` itself reporting an
error. Option 3 is the only one that (a) makes `pwc`/`pwl` self-document the actual
interpolation-style distinction directly in the name (constant vs. linear — the same "c"/"l"
distinction SPICE dialects don't need since they only ever had the linear one), (b) frees `pwl`
to mean exactly what it means everywhere else (real SPICE PWL semantics, matching
`general-mna`'s own source exactly, so a signal-domain reference and a `V`/`I` source built from
the same breakpoints are the same waveform), and (c) is a purely mechanical, behavior-preserving
rename for every existing use — every `internal-archive` netlist using the old
piecewise-constant block was renamed `kind=pwl` → `kind=pwc` and re-verified byte-identical
against its pre-rename baseline, not silently reinterpreted.

**Revisit if:** A future signal-domain source needs a third interpolation style (e.g. cubic/
spline breakpoints) and the one-keyword-per-style convention (`pwc`/`pwl`) stops scaling
cleanly — at that point, consider a single parameterized block (e.g. `kind=breakpoints
interp=constant|linear|cubic`) instead of adding a fourth bare keyword.

## `TimeStep::Adaptive` predicting discrete-block/gate events, added 2026-09-02

**Considered:**
1. Leave the adaptive controller purely LTE-driven: it only ever detects a gate change
   *after* a trial step has already landed past it (`gate_changed`, forcing a more robust
   integrator for that one step), never predicts ahead.
2. Have every `TimeStep::Adaptive` run default to a `dt_max` capped well below any block's own
   switching period, as a blanket safety margin, documented as a usage requirement.
3. Give `TimeStep::Adaptive`'s own per-step loop a predicted "earliest next event" clamp,
   computed in closed form from each in-scope block's *currently persisted* state (and its
   last-*evaluated* runtime inputs, for `PhaseShiftPwm`), applied to the trial `dt` *before* the
   LTE-driven retry loop runs at all.

**Chose:** Option 3.

**Because:** Option 1 is a real, previously demonstrated bug, not a hypothetical: an
`internal-archive` experiment (`experiments/elspice-pwl-ps-pwm-leg-modulator/`) ported
a phase-shift-PWM-with-dead-time modulator to `kind=pyblock` and measured real gate edges
**silently missed** (not just jittered) once `dt_max` approached the switching period — 0/100
missed at `dt_max=0.3x` the period, 30/100 at `0.48x`, 45/100 at `0.9x`, 57/100 at `3x`. If two
edges fall inside one trial step, only the "state differs from before" signal survives —
*both* transitions collapse into one detected change and a pulse vanishes outright. Option 2
pushes the burden onto every caller to know and manually enforce a margin below whatever
switching frequency their own netlist happens to use, is easy to violate silently (nothing
rejects a `dt_max` past the period, the corruption is just quietly there in the trace), and
doesn't help at all for a *closed-loop*-driven frequency that only becomes fast at run time.
Option 3 fixes the root cause directly, is strictly bounded in cost to at most one extra
closed-form evaluation per outer-loop iteration (`earliest_next_event_dt`,
`crates/dae-runtime/src/block_graph.rs`), and needs zero new public API, no `general-mna`
change, and no change outside `block_graph.rs`'s own adaptive loop and its already-in-scope
`evaluate_blocks` dispatch: `TimeStep::Fixed`, `lib.rs`'s own no-switch adaptive path (which
never calls `evaluate_blocks` at all, so it's structurally immune to this bug), and
`step_control.rs`'s own `lte_attempt`/`AdaptiveConfig` signatures are all untouched.

`BlockKind::Vco` is deliberately excluded from the event prediction (always contributes `None`)
— its own output is the raw phase itself (a continuous ramp in `[0,1)`), not a discrete/boolean
gate signal, so it has no "edge" a downstream comparator could silently swallow the way a missed
`Pwm`/`PhaseShiftPwm` transition is. `kind=octblock` stays separately, unconditionally rejected
under `TimeStep::Adaptive` (state-rollback reasons, unrelated to this feature) — this clamp
still computes an estimate for it (for whenever that restriction is eventually lifted) but is
unreachable in practice today.

Because `PhaseShiftPwm`'s `freq_command`/`phase_offset`/`duty` are runtime inputs (not fixed
construction params, unlike `Pwm`'s own `freq_hz`), the predicted edge is a same-input estimate,
valid only until the next step's inputs actually change — the same staleness caveat every other
closed-loop-driven prediction in this crate already has (e.g. `Pid`'s own anti-windup). Recomputing
fresh from the block's own currently-persisted state every accepted step (never extrapolating
multiple steps ahead) bounds that staleness to at most one step's own error, which is strictly
better than today's zero prediction, never worse — and the LTE-driven retry loop still runs
after the clamp and can shrink `dt` further; the clamp only ever tightens a trial step, it is not
a substitute for local-truncation-error control.

**Revisit if:** A future block-graph kind gains its own discrete "next transition" outside the
zero-order-hold `sample_time`/`PhaseShiftPwm`/`Pwm` shapes already covered — extend
`earliest_next_event_dt`'s own match rather than special-casing it elsewhere. If `kind=octblock`
is ever allowed under `TimeStep::Adaptive`, its event-clamp arm is already in place and should
just start being reachable.

## Streaming transient output instead of buffering the whole run, added 2026-09-04

`simulate_transient_with_blocks`/`_capped` (and the CLI's own predecessor to `run_transient_streamed`)
accumulated one `(t, OperatingPoint, outputs)` entry per accepted step into a `Vec` covering the
*entire* run, only printed/written after the whole simulation finished. This was fine for the
runs this project had actually tried until a real incident: a six-leg, frequently-soft-switched
circuit under `TimeStep::Adaptive` (see `internal-archive`'s
`elspice-pwl-tida-pi-pr-modulator-comparison` experiment) drove the development machine to
~11GB RSS / 24GB swap before being killed — entirely because of this buffering, independent of
whether the run itself was "stalled" (a real, separate bug fixed the same day, see the adaptive-
step-events entry above) or just genuinely needed several million steps (which it did, once that
bug was fixed and the adaptive tolerances were rescaled to the circuit's actual magnitude).

Fix: extracted the actual step loop into `simulate_transient_with_blocks_streamed`, which calls
an `on_step(t, point, outputs)` callback once per accepted step and holds nothing else —
`simulate_transient_with_blocks`/`_capped` are now thin wrappers that push into a `Vec` inside
their own callback, so their existing ~20 callers (mostly tests) see no behavior change at all.
The CLI's own transient path (`run_transient_streamed`) uses the streamed form directly: CSV rows
go straight to stdout as each step resolves, and raw-format rows go straight to a temp file (a
real SPICE rawfile's `No. Points:` header field has to be known before any row is written, so a
truly single-pass write isn't possible for that format — `raw_format::write_raw` was split into
`write_header`/`write_row` so the CLI can write rows to a temp file first, then once the real
point count is known, write the final file's header immediately followed by a plain byte copy of
the temp file, which is then deleted; proven byte-correct against real PySpice/spicelib readers,
see `tests/raw-output-python/`).

`kind=measure` still needs *some* history — a measurement needs the whole time series of
whichever signal(s) it names — but `measure::referenced_signals` walks every `MeasureKind`/
`EventCfg`/`ThresholdCfg` variant explicitly (no wildcard arm, so a new variant with its own
signal-name field won't silently go unaccounted-for) to find exactly which column names are
actually needed, and only those get a small `(Vec<f64>, Vec<f64>)` kept alongside the streaming
writer — a run measuring 2 signals out of 70 columns keeps roughly 2/70th of the memory a
full-`Vec` approach would, independent of the netlist's total column count. `evaluate_one`'s own
`samples_of` only ever looks up columns by name, so this "shadow" waveform (just `t` plus the
referenced names) behaves identically to the real, full one — a name that never matches any real
column is simply absent, giving the same "unknown signal" error a typo'd `out=`/`reference=`
already produced before this change, not a new failure mode.

**Revisit if:** the no-switch path (`simulate_transient`, `lib.rs`) is ever found to need the same
treatment — it wasn't touched here, matching the earlier adaptive-step-events fix's own scoping
(every real netlist in this project that could plausibly grow this large has ideal switches/
blocks and goes through `simulate_transient_with_blocks*`, not the no-switch path).

## Source material to adapt from
- `docs/journal/2026-08.md`, read end to end — the documentation-planning session's own Q&A
  entry, and everything before it, is largely a sequence of these decisions already reasoned
  through in narrative form; this chapter is the scannable, by-topic distillation of that.
