# Verification discipline

This project's central rule, stated in `AGENTS.md`'s "Verification discipline" section:
numerical agreement alone is not proof. For any new device model, LCP formulation, or
continuous block, the expected result must be derived independently — by hand, or from an
independently-worked KCL/KVL/transfer-function derivation — and *that* is what gets tested,
never just internal self-consistency. This chapter explains why that distinction matters, gives
a worked taxonomy of what "independent" actually means in this codebase, and documents the
concrete mechanism (`doc-verify/`) that enforces it for every piece of documentation this
project publishes, not just for new code.

## Why internal self-consistency isn't enough

A test that only checks a result against the same code path that produced it — or against a
plausible-looking number nobody derived separately — cannot catch a bug shared between the
implementation and the check. This is not a hypothetical risk; it is what actually happened in a
real converter deck in an internal validation-experiment archive, recorded in
a gotcha note on a Xyce boost-converter PI nonlinear integrator-windup failure. A boost converter's PI voltage
loop, run with aggressive gains, wound its integrator hard negative during startup overshoot
recovery because only an *upper* anti-windup clamp existed. Once the integrator collapsed below
zero the converter stopped switching entirely — and the output voltage that followed *looked*
like regulation. It was a capacitor discharging on its own time constant, converging toward
nothing in particular, but numerically it was a smooth, plausible-looking trace that would have
passed any check of the form "does the output settle near a reasonable value." Only comparing
against what the control loop should have been doing — an actively switching converter driving
toward a commanded setpoint, not an open-loop RC decay — exposed that the "final number" was
real but the mechanism producing it was wrong. That is the whole argument for this discipline in
one incident: a wrong model can still produce an unsuspicious-looking number, so the check has to
come from outside the model, not from staring harder at its own output.

## A taxonomy of "independent," with real examples

"Derive the expected result independently" is not one technique — this codebase actually uses
several, chosen per situation:

- **Hand-derived closed form.** Worked out with pencil and paper (or a symbolic tool used only
  as a calculator, never as the thing being tested) before the code that should reproduce it is
  run. `crates/lcp-solver/tests/fixtures.rs` is the canonical example: six LCP fixtures, each
  with an expected solution worked out independently of the solver, including a degenerate
  boundary case and a genuinely infeasible LCP that must return `LcpError::RayTermination`
  rather than fabricate an answer. `crates/pwl-devices/tests/two_ideal_diode_circuit.rs` is the
  same idea at circuit scale: the PCNR paper's two-diode-plus-resistor circuit, folded into an
  LCP by hand and checked at two operating points computed before the code existed
  (`docs/journal/2026-08.md`, 2026-08-17). Netlist-level `ic=` initial-condition tests follow the
  identical pattern — `crates/dae-runtime/tests/netlist_initial_conditions.rs`'s expected
  voltages and currents are computed from the first backward-Euler step *by hand*, before the
  test was run, not read off a previous run's own output (`docs/journal/2026-09.md`,
  2026-09-11 06:29).
- **Convergence-order checks against a converged reference.** Not "smaller error at a smaller
  `dt`" — that alone is consistent with almost any correctly-*shaped* but wrongly-*scaled* bug.
  The actual check is that the error shrinks at the rate the integrator's own order predicts:
  backward Euler roughly halves its error as `dt` halves (1st order), trapezoidal roughly
  quarters it (2nd order) — see `docs/architecture.md`'s "Status" section for where this is
  verified for the transient loop itself. A test that only checked "the error got smaller" would
  pass for an integrator silently running at the wrong order; checking the *rate* is what
  actually distinguishes "correct" from "merely converging."
- **Cross-checking a fused/convenience implementation against composing its own primitives.**
  `clarke_park` (the fused abc→dq0 transform) is checked against calling `clarke` and then `park`
  separately and comparing (`crates/continuous-blocks/src/coordinate_transforms.rs`,
  `fused_matches_composed`-style tests) — see `book/dev-guide/src/vector-signals.md`'s own
  citation of `CoordinateTransform`. This is not a redundant check: composing the two primitives
  independently re-derives the same math through a structurally different code path, and this
  exact style of check is what this codebase's own history shows catches a real bug class — a
  swapped d/q axis convention is invisible to a test that only exercises the fused function
  against itself, because a self-consistent swap still agrees with a self-consistently-swapped
  reference.
- **Cross-checking against a previously validated Xyce/ngspice baseline.** `AGENTS.md` names
  this explicitly: compare end-to-end transient results against the already-validated Xyce runs
  captured in an internal validation-experiment archive's converter-benchmark and
  dual-active-bridge (DAB) experiment folders wherever a matching topology exists. These are real, previously
  cross-checked baselines from an independent simulator using an entirely different numerical
  method (Newton-Raphson with voltage limiting, not this project's LCP mode selection) — the
  strongest available check for anything with a topology those experiments already cover, since
  agreement between two structurally unrelated solvers is much harder to fake than agreement
  between a solver and its own test suite.

## The `doc-verify/` mechanism: enforcing this for documentation, not just code

The taxonomy above is about verifying new *implementation*. `AGENTS.md`'s "Documentation"
section extends the same discipline to every claim this project publishes about a netlist
component: **"Testing every documented Example and Errors claim against a real run of the CLI,
before writing the doc comment, is mandatory practice, not optional polish."** The mechanism
that enforces this is `doc-verify/<kind>/` — see `.claude/skills/write-component-doc/SKILL.md`
for the full authoring workflow; this section covers why the mechanism exists and what it looks
like from the outside.

Every `kind=` component's reference entry (generated from doc comments in
`general-mna/src/block_graph.rs` into `book/user-guide/src/component-reference.md`, the same way
`cargo doc` generates from source rather than being hand-maintained) is backed by a **committed**
`doc-verify/<kind>/` folder — real, checked-in source, not scratch work, reviewed like any other
change (only build artifacts compiled *from* it — `.so`/`.pyc`/`__pycache__` — are gitignored).
Each folder contains a real `.cir` fixture per documented `## Example` and per distinct
`## Errors` claim, and a `test_<kind>.py` that actually subprocesses the real
`general-simulator-cli` binary and asserts the *specific* claimed value or the *specific*
observed error text — not just "exit code 0," and not a paraphrase of what the source code is
expected to print. This closes exactly the gap the hand-derivation taxonomy above closes for
implementation code: a doc comment's claim, read only by eye against the source, can drift from
what the compiled binary actually does (a changed error-message format, a validation path that
turns out to be unreachable, a plausible-looking example netlist that was never actually run).
Running it is what makes the claim evidence instead of an assertion.

`doc-verify/pid/` is a representative worked example (`doc-verify/pid/README.md`):

- `example.cir` — the doc comment's own `## Example` netlist, verbatim, run and checked: with
  `kp=1 ki=0 kd=0` against a fixed zero-error signal, `PID1`'s output must stay exactly `0` every
  step.
- `nonzero_error.cir` — a second fixture proving `kp` is genuinely *applied*, not silently
  ignored: `ERR=0.5`, `kp=0.3`, clamp bounds wide enough never to engage, expected output exactly
  `0.15`.
- `error_nonpositive_n.cir` — reproduces the documented `NonPositiveFilterCoefficient` error
  (`n=0`) and asserts the real observed message.
- `error_unpaired_dynamic_clamp.cir` — reproduces the documented clamp-pairing error
  (`clamp_lo_in` given without `clamp_hi_in`) and asserts its real observed message.

`doc-verify/sig2phys/` shows the same discipline catching a real, previously silent bug: its
`error_converter_wired_as_a_node.cir` fixture reproduces a construction that used to *succeed*,
silently reporting `V(VDRV) = 0` for a `Sig2Phys` converter wired into the circuit as an ordinary
node instead of being referenced by name (`docs/journal/2026-09.md`, 2026-09-04) — exactly the
kind of "plausible but wrong" numerical output the boost-PI incident above illustrates at circuit
scale, caught here at the single-block level by insisting the claimed error path be reproduced,
not just read off the source.

A doc comment whose claims were never run this way is not a finished entry, regardless of how
correct it looks on inspection — the same principle as the hand-derivation taxonomy above, applied
to documentation instead of implementation: a plausible-looking claim is not evidence until
something outside the claim itself has checked it.
