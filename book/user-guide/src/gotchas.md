# Troubleshooting and gotchas

One entry per real trap, each short enough to Ctrl-F by its error message or symptom. These
are the failures a *user* of the CLI/netlist grammar hits; the internal
numerical/pivoting/testing traps a *contributor* hits live in the dev guide's
[Journal and gotchas conventions](../dev-guide/journal-and-gotchas.md) chapter and in
`docs/gotchas/` — this page is the user-facing subset.

## `GateTargetNotSig2Voltage` — `ctrl=` must name a `domain=voltage` `sig2phys` converter

**Symptom:** a validation-stage error naming the gate target as not a `sig2phys`
voltage converter.

**Cause:** every ideal switch's gate is `gate=block ctrl=<name>`, and `<name>` must
specifically be a `kind=sig2phys domain=voltage` converter — never a raw `pid`/`vco`/
`hysteresis` block directly, and never a `domain=current` converter. An ideal switch's gate
is itself a voltage, so it shares the same converter a `V`-source's own magnitude uses.

**Fix:** put `kind=sig2phys domain=voltage in=<your block>` in between. See
[Gate bindings](gate-bindings.md), "Two real errors worth knowing before you hit them."

## `Sig2PhysUsedAsCircuitNode` — a converter's name used as a circuit node

**Symptom:** a hard, up-front error (both `--mode dc` and `--mode transient`) naming the
converter, the offending element, and the node token.

**Cause:** a `kind=sig2phys` block has no terminals and stamps nothing — it can only be
*referenced by name* (a source's value field, or `ctrl=`), never wired as a node. Writing
`R1 VDRV 0 1k` where `VDRV` is a converter's name used to build and solve without complaint
while silently reading `V(VDRV) = 0` (an undriven node) next to the block's own correct
nonzero output column.

**Fix:** name the node something else. Reproduced by
`doc-verify/sig2phys/error_converter_wired_as_a_node.cir`.

## Ideal-switch node order is `(drain, source)`

**Symptom:** a low-side switch conducts when it should block (or vice versa) — the body
diode is effectively backwards.

**Cause:** the device card declares its two terminals in plain SPICE-conventional
`(drain, source)` order. An earlier version of the model required `(source, drain)` instead;
6 of 8 switches in the TIDA-010954 cycloconverter experiment were declared in the natural
order and had their body diodes backwards before the contract was changed to remove the
footgun.

**Fix:** declare `(drain, source)`. Full history in
[PWL devices](pwl-devices.md), "The node-order convention: `(drain, source)`."

## Every ideal switch in one run must share `r_on`

**Symptom:** `all ideal switches must share the same r_on ...` (rejected at build time when
two declarations disagree).

**Cause:** `dae-runtime`'s switch mechanism uses one shared on-resistance per call
(`dae_runtime::solve_dc_with_ideal_switches`'s doc comment). Per-instance `r_on` is a known
limitation, tracked in the dev guide's [Open extension points](../dev-guide/open-questions.md),
not a netlist typo being silently tolerated.

**Fix:** give every `kind=ideal_switch` line the same `r_on`. See
[Grammar overview](netlist-grammar.md), "The shared `r_on` constraint."

## `--mode dc` rejects block-driven gates

**Symptom:** a `--mode dc` run of a netlist containing ideal switches fails with an error
naming the device, rather than producing an operating point.

**Cause:** every gate is block-driven, and a DC operating point has no notion of the
time-stepped state a `pid`/`vco`/`pwm`/`pspwm` block carries. The failure is explicit — the
device is not run with some silently-assumed fixed gate state.

**Fix:** run `--mode transient`. See [Command-line flags](cli-reference.md).

## `CScriptRequiresCloneForAdaptiveStep` — adaptive stepping needs `cscript_clone`

**Symptom:** a `kind=cscript` block fails at the very first step under adaptive stepping
(the default when `--dt` is omitted) with
`CScriptRequiresCloneForAdaptiveStep { block_name: "<name>" }`.

**Cause:** adaptive stepping clones every block's state before each trial step and discards
the clone on a rejected trial; an opaque C state pointer can't be deep-copied without the
library's own help.

**Fix:** export `cscript_clone` from your library, or run with a fixed `--dt`.
`doc-verify/cscript/error_adaptive_needs_clone.cir` reproduces it. See
[The CScript escape hatch](cscript.md).

## `OctBlockDoesNotSupportAdaptiveStep` — `kind=octblock` never runs adaptively

**Symptom:** an `octblock` netlist is rejected outright whenever adaptive stepping is
requested, with `DaeError::OctBlockDoesNotSupportAdaptiveStep`.

**Cause:** Octave-side state lives in the one shared `octave-cli` session, which is never
cloned — a rejected trial's mutations would already be committed, so there is no
`cscript_clone`-style opt-in that could make it safe. This one is architectural and
permanent, unlike the `cscript` case above.

**Fix:** run with a fixed `--dt`. See
[Fixed vs. adaptive time stepping](time-stepping.md).

## Fixed PID clamp silently fails to engage during a startup transient

**Symptom:** sustained, hard-to-diagnose windup-driven oscillation, even though the
controller's output visibly saturates far below the configured clamp.

**Cause:** a `clamp_lo=`/`clamp_hi=` bound sized for the *final* steady-state range is badly
oversized early on — the canonical case is a current-loop PID commanding a pole voltage that
can't physically exceed roughly half a DC bus voltage that is itself still rising during a
soft-start ramp — so anti-windup never engages even though the real plant is already
saturated.

**Fix:** use `clamp_lo_in=`/`clamp_hi_in=` when the achievable range is itself
state-dependent — the dynamic clamp reads the bound fresh from the graph every step. See
[Dynamic blocks](dynamic-blocks.md), "`kind=pid`: fields and anti-windup."

## `UnknownBlockInput` vs. `AlgebraicLoop`

**Symptom:** one of two errors naming a block.

**Cause and fix:** they are different problems with different fixes — the distinction is
worth learning once:

- `UnknownBlockInput` — the named block **doesn't exist anywhere** in the graph (a typo, or
  a genuinely undeclared name). It is never a same-step-ordering problem: evaluation order
  is derived automatically from the dependency graph, so a block may reference another
  declared anywhere in the file, before or after it. Fix the name.
- `AlgebraicLoop { cycle: [...] }` — a genuine same-step cycle among `Signal::Block`
  references, reported with the exact closing path (`["A", "B", "A"]`, a self-reference
  reports `["A", "A"]`). Fix: route one edge in the printed cycle through `prev:` — the
  one-sample delay that turns a same-step loop into a legitimate sampled-data feedback path.

See [Signals](signals.md), "What happens if you write a same-step cycle anyway."

## Source material this was adapted from

- `book/user-guide/src/pwl-devices.md`, `netlist-grammar.md`, `gate-bindings.md`,
  `cli-reference.md`, `cscript.md`, `time-stepping.md`, `dynamic-blocks.md`, `signals.md` —
  each entry above cites its own section.
- `docs/gotchas/GOTCHA-001-cscript-ffi-fixture-race.md` — read and deliberately *excluded*:
  a contributor-facing test-harness race, documented in the dev guide's journal-and-gotchas
  chapter instead.
