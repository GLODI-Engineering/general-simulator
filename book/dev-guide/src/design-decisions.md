# Design decisions log

A running, dated, by-topic log of "we considered X, chose Y, because Z" entries — the scannable
counterpart to `docs/journal/`, which is chronological narrative recording *what happened, in
order*. This chapter instead groups by topic and keeps only the decision itself: what was
considered, what was chosen, why, and (where known at the time) what would reopen the question.
Reading the journal end to end tells you the story of how this project got here; reading this
chapter tells you, for a given design question, what was decided and why without having to
reconstruct it from the narrative. Some entries below are still open questions resolved
mid-project and folded in as a settled decision once the resolution shipped; others were written
up directly from an already-resolved journal discussion. Candidates not yet written up as their
own entries remain noted inline in the source material this chapter draws from
(`docs/journal/2026-08.md`, `docs/journal/2026-09.md`) and in `open-questions.md` where the
question is still genuinely open rather than merely undocumented.

Format: one entry per decision, `## <short title>`, `**Considered:**`, `**Chose:**`,
`**Because:**`, `**Revisit if:**` (conditions that would reopen the question, where known).

## Physical/signal-domain converters

**Considered:**
1. Leave circuit-quantity reads and gate targets as bare, unmarked references —
   `Signal::Measure` reading any node/branch directly, and any `GateBinding` naming any control
   block's raw output, exactly as the block graph worked before this decision.
2. Enforce the physical (circuit)/signal (block-graph) domain boundary only as a documentation
   convention or a UI-level wiring rule (e.g. a GUI front-end refuses to draw the wire), leaving
   the netlist grammar itself and `dae-runtime` permissive.
3. Enforce the boundary at the netlist/`dae-runtime` level with dedicated converter block kinds
   — `kind=probe` for physical-to-signal reads, `kind=sig2gate`/`sig2v`/`sig2i` for signal-to-physical
   writes and gate actuation — so that a violation is a build-time error regardless of which tool
   (hand-written netlist, GUI export, script) produced the netlist.

**Chose:** Option 3, requested explicitly by the user after the 2026-08-21 robustness Q&A
(`docs/journal/2026-08.md`, "Robustness Q&A" entry) as "the same rule a real block-diagram tool
enforces with its own physical/signal converter blocks, applied here at the netlist level rather
than a GUI's wiring canvas."

**Because:** Option 1 makes every `GateBinding` and every circuit-quantity read a silent,
un-typed coupling between two domains this project otherwise treats as structurally different —
the circuit domain resolved by the LCP/DAE fold, the signal domain evaluated by
`evaluate_blocks`'s own topological pass — with nothing stopping a netlist author from wiring one
into the other by accident. Option 2 pushes enforcement onto a layer this project doesn't fully
control (a GUI is one possible netlist producer among several, including hand-written text and
scripts) and, per `AGENTS.md`'s "Project boundaries," `general-mna`/`dae-runtime` are the actual
source of truth for what a netlist means — a convention enforced only outside them is exactly the
kind of unenforced-by-construction rule this project's own later `Sig2Phys`-wired-as-a-node fix
(`docs/journal/2026-09.md`, 2026-09-04) shows can silently produce a wrong-but-plausible answer
(`V(VDRV) = 0`) instead of an error. Option 3 also closed a real, previously documented gap
noted in an internal buck-converter/DC-motor cascade experiment's README
— "there is no path for a block to inject current back into the MNA system" — by adding the
write-direction converters (`sig2v`/`sig2i`) alongside the read direction, and needed **zero
changes** to the sibling `general-mna` repo: `Expression::parse_scalar` already accepted a bare
symbol as a source's literal, and `MnaSystem::evaluate` already substituted any symbol present in
an expression tree generically, both confirmed by direct reading of `general-mna`'s own source
before relying on them, not assumed.

Implementation caught one real correctness bug before it shipped: `Scheme::Trapezoidal`'s
`u_prev` history-averaging term previously skipped re-evaluation whenever
`system.transient_sources.is_empty()`, which is also true for a block-driven source (it isn't a
`TransientFunction` at all) — this would have silently used the *current* step's block output as
if it were the *previous* step's in the trapezoidal average. Fixed by also gating on
`extra_values_prev.is_empty()` (`crates/dae-runtime/src/lib.rs`).

**Revised 2026-08-27**: `Sig2Gate` was removed and merged into `Sig2Voltage` — an ideal switch's
gate is itself a voltage, not a distinct discrete-actuation signal domain, so no dedicated
gate-only converter kind is needed. `DaeError::GateTargetNotSig2Gate` was renamed
`GateTargetNotSig2Voltage` to match; two converter kinds remained (`Sig2Voltage`/`Sig2Current`),
not three.

**Revised 2026-09-01 (two changes):** `Sig2Voltage`/`Sig2Current` were themselves merged into one
`BlockKind::Sig2Phys { domain: PhysicalDomain }` variant (`domain=voltage`/`domain=current`) —
same rationale as the `Sig2Gate` merger: the two were identical in shape and evaluation, differing
only in which device letter/`GateBinding` target they're allowed to satisfy, now expressed as a
field instead of a second enum variant. `DaeError::GateTargetNotSig2Voltage`/
`SourceNotSig2PhysicalConverter` kept their names since the enforcement semantics were unchanged;
only the netlist syntax (`kind=sig2phys domain=<voltage|current> in=<signal>`) and the underlying
Rust type changed. Separately, `kind=probe` (`BlockKind::Probe`/`ProbeTarget`) was renamed
`kind=phys2sig` (`BlockKind::Phys2Sig`/`Phys2SigTarget`) for naming symmetry with `sig2phys` —
`phys2sig` reads a circuit quantity into the signal domain, `sig2phys` drives a signal-domain
value onto a circuit quantity. Pure rename in both cases: fields, validation, and behavior
unchanged; no back-compat alias for the old `probe` name.

**Revisit if:** a third physical domain (e.g. a genuinely distinct actuation signal beyond
voltage/current) needs its own converter semantics — extend `PhysicalDomain` rather than adding a
new bare `BlockKind` variant, matching how `Sig2Gate` was folded into `Sig2Voltage` rather than
kept as a separate case.

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
every existing reference-schedule netlist in this project's internal validation-experiment
archive (e.g. a speed-step schedule `REF kind=pwl points=[[0,800],[0.05,1500]]`) relies on the
*step* behavior — reinterpreting it as a ramp would silently corrupt every one of those
experiments' own recorded results without a single line of `general-simulator` itself reporting an
error. Option 3 is the only one that (a) makes `pwc`/`pwl` self-document the actual
interpolation-style distinction directly in the name (constant vs. linear — the same "c"/"l"
distinction SPICE dialects don't need since they only ever had the linear one), (b) frees `pwl`
to mean exactly what it means everywhere else (real SPICE PWL semantics, matching
`general-mna`'s own source exactly, so a signal-domain reference and a `V`/`I` source built from
the same breakpoints are the same waveform), and (c) is a purely mechanical, behavior-preserving
rename for every existing use — every netlist in the internal validation-experiment archive using
the old piecewise-constant block was renamed `kind=pwl` → `kind=pwc` and re-verified byte-identical
against its pre-rename baseline, not silently reinterpreted.

**Revisit if:** A future signal-domain source needs a third interpolation style (e.g. cubic/
spline breakpoints) and the one-keyword-per-style convention (`pwc`/`pwl`) stops scaling
cleanly — at that point, consider a single parameterized block (e.g. `kind=breakpoints
interp=constant|linear|cubic`) instead of adding a fourth bare keyword.

## PWL segments via LCP vs. Newton-Raphson + voltage limiting

**Considered:**
1. Follow the standard SPICE-family approach: solve `g(x) = 0` (KCL/KVL plus device equations)
   via Newton-Raphson, with per-iteration voltage limiting to keep exponential device curves
   (e.g. a real diode's $I = I_S(e^{V/V_T} - 1)$) from diverging.
2. Model every nonlinear device as piecewise-linear (PWL) from the start, and resolve which
   discrete combination of segments is active, once per timestep, as a Linear Complementarity
   Problem (LCP) — no continuous iteration at all.

**Chose:** Option 2 — this is this project's founding premise, not a decision made partway
through (`docs/architecture.md`, "Why not Newton-Raphson," written at repo scaffolding,
2026-08-17).

**Because:** Voltage limiting works but breaks the clean `g(x)=0` abstraction Newton-Raphson
depends on: `g` and its Jacobian become functions of iteration *history*, not just `x`, which is
inconsistent between devices sharing a node and incompatible with most modern nonlinear-solver
enhancements — documented in detail against the primary Xyce source in an internal write-up
analyzing Xyce's Newton-Raphson formulation and its voltage-limiting behavior. If every device is genuinely
piecewise-linear, the circuit is *exactly* linear within any fixed combination of active
segments, so "iterate Newton on continuous device physics" can be replaced with "resolve, once
per timestep, which discrete combination of segments is active" — a fundamentally different,
non-iterative-in-the-Newton-sense problem, well-posed as an LCP because every PWL diode segment
shares one fixed reference conductance (`docs/architecture.md`, "Mode selection as a Linear
Complementarity Problem"). This was validated, not just argued, before any circuit-facing code
existed: `crates/lcp-solver`'s own fixtures were verified against hand-solved cases first
(Milestone 1), then the PCNR paper's two-diode circuit was folded into an LCP by hand and solved
both by hand and by the real solver, matching to `1e-6` with "no Newton-Raphson anywhere"
(`docs/journal/2026-08.md`, 2026-08-17 09:53) — the first end-to-end proof the whole approach
reproduces a real circuit's answer.

**Revisit if:** a device genuinely cannot be modeled as piecewise-linear to acceptable fidelity
for some future use case — this project's own scope (PWM power converters with ideal/PWL
switches and diodes) has not hit this limit; a BSIM-style real-transistor model is explicitly
named as a distinct, not-yet-implemented future device kind rather than a reason to revisit this
choice (`docs/journal/2026-08.md`, 2026-08-29 17:57, the `Mosfet` → `IdealSwitch` rename entry).

## Ideal-switch channel state: exogenous command vs. LCP-resolved

**Considered:**
1. Fold ideal-switch channel state (on/off) into the same LCP the diode segments already share,
   so a single simultaneous solve resolves both diode segments and switch state together.
2. Treat the gate command as an external, exogenous input — decided by the caller (a PWM
   comparator, a block-graph `GateBinding`) before the circuit is even built — and pick one of
   two fixed structural stamps (closed: plain `r_on` resistor; open: intrinsic body diode,
   itself folded into the diode LCP) accordingly.

**Chose:** Option 2 (`docs/journal/2026-08.md`, "Robustness Q&A," Q5, 2026-08-21 11:33 — asked
as a direct challenge by the user before trusting the simulator enough to publish it).

**Because:** the diode LCP works specifically because every segment of a diode shares one
*fixed reference conductance* `g_off` — segment choice only ever shifts an affine current-source
term at *fixed* topology, keeping the linear system `A0` genuinely fixed regardless of which `z`
comes back nonzero. A MOSFET channel going from closed to fully open is not that kind of change:
it is the literal appearance/disappearance of a conductance path, a *topological* change to `A0`
itself, so the algebraic trick the diode fold depends on does not apply — this is exactly why
`build_with_mosfets`/`build_with_ideal_switches` rebuild the symbolic system on a gate flip
rather than folding switch state into the fixed-`A0` diode LCP. Independent of the math, the
physics also supports it: in every circuit this crate targets (PWM power converters), the gate
signal genuinely *is* an externally-commanded control input in the real system too — a real gate
driver does not discover it should switch by solving a complementarity condition on `Vgs` vs.
`Vth`, a controller commands it directly, so treating it as exogenous is model fidelity, not a
shortcut.

**Revisit if:** a genuinely self-triggering ideal switch is needed — a relay, a fuse, or a real
MOSFET operated with no active gate drive (pure natural commutation) — where switch state
*should* be resolved from circuit state. The user agreed during the Q5 discussion this is a
different, real feature needing its own formulation (the same topology-change problem the diode
fold doesn't have), not "the same LCP done better," and asked to record it as an open extension
point rather than build it under time pressure — see `open-questions.md`, still unimplemented as
of this writing.

## Block-graph execution order: derived topological sort vs. manual declaration order

**Considered:**
1. Keep evaluating blocks in netlist declaration order, requiring the netlist author to declare
   every block after everything it depends on (the state at the time this question was asked —
   `block_graph.rs`'s own doc comment said so explicitly, and `Signal::Block` required its target
   "declared earlier in the same slice").
2. Derive execution order automatically from the actual `Signal::Block` dependency graph via a
   real topological sort, computed once per run before any step solves, so declaration order
   stops mattering.

**Chose:** Option 2 (`docs/journal/2026-08.md`, "Robustness Q&A," Q1/Q2, 2026-08-21).

**Because:** Option 1 is a manual discipline imposed on the netlist author with no enforcement —
nothing checked that a block's dependencies were actually declared earlier, so a violation would
surface only indirectly. It also had no way to distinguish "this name doesn't exist" from "this
name exists but hasn't been evaluated yet in the current one-pass-forward evaluation," both of
which collapsed into the same opaque `DaeError::UnknownBlockInput`. `topological_order`, added in
the same session, uses three-color DFS marking: a genuine same-step algebraic loop (a back-edge
into a Gray node) is reconstructed into its exact closing path (`["A","B","C","A"]`) and reported
as the new, distinct `DaeError::AlgebraicLoop`, while `UnknownBlockInput` now genuinely only means
"this name doesn't exist anywhere." `Signal::Measure`/`Signal::BlockPrev` were confirmed to never
contribute a dependency edge (both read state fixed before the step starts), so they remain the
sanctioned way to close what would otherwise be a same-step loop — this is also why
`Signal::BlockPrev` (see below) was the right tool for the self-referencing cases that motivated
it, rather than a special-cased cycle-resolution rule inside the topological sort itself.

**Revisit if:** a use case needs a same-step cycle resolved by something other than a one-step
delay (e.g. an algebraic loop with a genuine simultaneous solution) — not attempted; every
self-referencing case encountered so far (SRF-PLL angle estimate, PMSM current-controller
feedback) is correctly modeled as a one-sample delay, matching real digital-controller behavior,
not an approximation of a truly-simultaneous system.

## `Signal::BlockPrev`: a block reading its own previous output

**Considered:**
1. Leave the block graph feedforward-only within a step (`Signal::Block`, an error on an unknown
   same-step name) plus `Signal::Measure` for the *circuit's* previous state — no mechanism for a
   block to read its *own* previous output at all.
2. Add `Signal::BlockPrev(String)`: reads a named block's own output from the *previous* step
   (`0.0` before the first step, matching every dynamic block's "starts at rest" convention), the
   direct block-graph counterpart to `Signal::Measure`.
3. Allow a same-step self-reference directly, resolved by a special same-step cycle-resolution
   rule (e.g. fixed-point iteration within a step) rather than a one-step delay.

**Chose:** Option 2 (`docs/journal/2026-08.md`, 2026-08-20 22:46).

**Because:** designing the PFC/PMSM-drive experiments this session's work was building toward
surfaced a real structural gap neither Option 1 covered: an SRF-PLL's own angle estimate feeds
the very `Park` block used to compute the error that updates that estimate, and a PMSM current
controller regulates a `Pmsm` block's own `id`/`iq` outputs by commanding that same block's
`vd`/`vq` inputs — both need a block to read its own previous output, and every existing
`Signal` variant was either feedforward-only within a step or read the *circuit's* state, never a
block's own. Option 3 was rejected without being built: it would reopen exactly the
same-step-cycle ambiguity `topological_order`'s algebraic-loop detection (see above) was just
built to catch and report as an error, trading a clear diagnostic for a special-cased resolution
rule. Option 2 instead gives the one-sample delay every real digital controller reading its own
last output already has — matching the "sampled-data co-simulation" framing `block_graph.rs`'s
own module doc comment uses. Verified with the simplest possible exercise of the pattern
(`tests/block_prev_signal.rs`): a `Sum` block computing `ACC = prev:ACC + 1` every step must
equal exactly `n` after `n` steps, checked at every step, not just the last, plus a check that an
unresolved `prev:` name is `0.0` (not an error) — deliberately distinct from `Signal::Block`'s
hard-error behavior on an unknown same-step name, since a not-yet-existing previous value is a
normal condition (the first step), not a mistake.

**Revisit if:** a future use case needs the delay itself to be something other than exactly one
circuit step (e.g. a block reading its own state from `N` steps ago) — not needed by any case so
far.

## Per-block RK4 vs. one fused ODE across the whole block graph

**Considered:**
1. Integrate every dynamic block (`Pid`/`StateSpace`/`TransferFunction` via `rk4_step`; `Pmsm`
   via its own bespoke `step()`) with its own independent RK4 call, once per circuit step, using
   only already-resolved input numbers from earlier in the topological order.
2. Fuse every dynamic block's state into one large ODE, stepped by a single RK4 call across the
   whole block graph per circuit step.

**Chose:** Option 1, confirmed already in place and correct, not changed
(`docs/journal/2026-08.md`, "Robustness Q&A, Q6," 2026-08-21 14:21).

**Because:** `evaluate_blocks` already runs blocks in `topological_order` (see above), so by the
time any block is stepped, every `Signal::Block` input it needs is already a concrete
number resolved *this* step — `rk4_step`/`Pmsm::step` hold that input vector fixed across all
four RK4 stages, exactly what a real digital controller's own discrete update does (read inputs
once, integrate forward, done), not an approximation of a continuously-coupled system. Nothing
needs a block's mid-step (`k1`/`k2`/`k3`) trial state, only its already-fixed inputs, so there is
no possible entanglement between two blocks' RK4 stages the way there would be if, say, a
genuinely bilinearly-coupled system were split across two separate blocks each needing the
*other's* current-step output — that would be a real same-step algebraic loop (see "Block-graph
execution order" above), not solvable by any per-block integrator alone. `Pmsm` is the case that
makes this concrete: its `id`/`iq`/`omega_m` dynamics are genuinely bilinearly coupled (back-EMF/
cross-coupling and torque terms mix multiple states), the one place a fused treatment might seem
tempting — but the fix is not fusing `Pmsm` with anything else; `Pmsm::step()` evaluates its
*entire* nonlinear vector field inside one call, so RK4's own intermediate stages see the coupled
dynamics correctly, the same way any single nonlinear-ODE integrator handles a coupled system.
The coupling lives inside the vector field, not across the block graph.

**Revisit if:** a future block genuinely needs another block's *mid-step* (not just previous- or
current-step) state to compute its own derivative — not encountered; every coupled case so far
(`Pmsm`) is handled by keeping the coupled dynamics inside one block's own vector field rather
than splitting them across blocks. See `block-graph-rk4.md` for the fuller narrative treatment
once written.

## Bench-scale vs. mains-scale for the three-phase PFC switching demo

**Considered:**
1. Build the Stage 2 (real six-ideal-switch, three-phase active-front-end) switching bridge at
   full mains ratings (325V peak phase / 700V DC bus target) from the start.
2. Scale down to a deliberate bench-scale proof-of-concept (50V peak phase, 120V DC bus target)
   once mains-scale startup revealed a real problem, rather than solve the mains-scale problem
   directly.

**Chose:** Option 2 (an internal three-phase PFC experiment's README,
"Stage 2 — real switching bridge," session started 2026-08-20).

**Because:** at the original mains scale, the AC-side R-L branch provided very little natural
current limiting against a stiff grid with near-zero initial bus counter-voltage — real hardware
solves this with dedicated precharge circuitry, explicitly out of scope for this experiment.
Scaling down to bench values was recorded as "a deliberate simplification, not a full-ratings
result," made explicitly as a decision rather than discovered as an accidental limitation.

A second, related decision from the same experiment is worth recording alongside it: once three
real bugs (duty normalization, startup inrush, and a fixed anti-windup clamp vs. a dynamically
achievable range) were found and fixed, the closed loop still did not converge — commanded-zero
`q`-axis current grew to tens of amps instead of settling, and the DC bus oscillated rather than
tracking its soft-start ramp, even though the synchronization chain itself remained provably
correct throughout (`PRK`/`PRK_q` held to full floating-point precision even while the current
loop diverged around it). The user explicitly chose to leave this open rather than force a fix
under time pressure, on the principle that a decision to stop and record what's known beats
guessing further fixes blind. This is recorded here as a decision (stop, document, hand off) even
though the underlying bug (see `open-questions.md`) stayed open.

**Revisit if:** the q-axis divergence above is root-caused — likely candidate per the experiment's
own README is verifying the decoupling/feedforward algebra against the bridge's *real* transfer
function (which includes switching/sampling delay) rather than the idealized one Stage 1's
reduction assumed.

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

**Because:** Option 1 is a real, previously demonstrated bug, not a hypothetical: an internal
phase-shift-PWM leg-modulator experiment ported
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
circuit under `TimeStep::Adaptive` (see an internal phase-shift-PWM modulator comparison
experiment) drove the development machine to
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
