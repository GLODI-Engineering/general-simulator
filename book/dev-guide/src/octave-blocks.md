# Octave blocks (`kind=octfunc`, `kind=octblock`): design

*(`kind=octfunc` implemented first, as a genuinely separate, additive block kind alongside
`pyfunc` — the direct `octave-cli`-hosted analog, not a mode of anything else, mirroring
`pyfunc-blocks.md`'s own before/after structure. `kind=octblock` — the stateful sibling,
`pyblock`'s own Octave-hosted analog — implemented in a later session on top of the same
`octave_ffi::OctaveSession`; see "Stateful blocks (`kind=octblock`)" below for what it adds.)*

## What this is, and what it isn't

`kind=octfunc` is the stateless-function analog of `kind=pyfunc` (see `pyfunc-blocks.md`), for
people with legacy Octave-compatible `.m` scripts instead of Python: a plain, named
function, called once per resolved step with each declared `inputs=` entry as its own positional
argument, no `start`, no persistent `state`, no `t`/`dt` boilerplate, no `xc_count` — there is
nothing for any of those to mean for a function with no memory between calls. Netlist usage:

```text
SRC kind=const value=3
G1 kind=octfunc path=add_one.m function=add_one in=SRC
```

where `add_one.m` is:

```text
function y = add_one(x)
  y = x + 1;
end
```

## Why a subprocess, never a linked library — the licensing constraint

GNU Octave is GPLv3-licensed. Linking Octave's embedding library (`liboctinterp`) directly into
this project's own permissively-licensed binaries would risk pulling them under GPL. **This
crate never links against any Octave C/C++ library, and has no Octave crate/library as a
build-time dependency at all** — the only integration is spawning the separately installed
`octave-cli` *binary* as a subprocess. This is "mere aggregation" under GPL, not linking — the
same reasoning that lets tools shell out to `ffmpeg`/`gs` without inheriting their license (this
workspace's own `internal-archive/gotchas/ngspice-xspice-codemodel-needs-mfbinit-env.md`
records an analogous already-used external-tool pattern, for a different reason).

This decides the crate's own shape as much as the protocol does: `octave-ffi`, unlike
`pyblock-ffi` (which needs `pyo3`+`libpython` at *build* time, and is gated behind an optional
Cargo `python` feature for exactly that reason), has **no special build-time dependency at all**
— it's pure `std::process::Command` subprocess management, builds unconditionally on every
platform and configuration, and only fails at *runtime*, with a clear
[`octave_ffi::OctaveError::NotFound`], if `octave-cli` isn't on `PATH` when a `kind=octfunc`
block is actually used. `dae-runtime`'s own `Cargo.toml` takes `octave-ffi` as an ordinary,
unconditional dependency — there is no `octave` feature flag mirroring `python`, and deliberately
so; adding one would copy `pyblock-ffi`'s feature-gating pattern for a reason (a build-time
native dependency) that simply doesn't apply here.

## One shared, persistent process — and the numbers behind that choice

Every `kind=octfunc` block instance in a single simulation run shares **one** `octave_ffi::OctaveSession`
— exactly the `kind=pyfunc` case (stateless, no per-instance state to isolate), so there's
nothing that needs a *separate* process per block instance, and sharing is what makes a single
`octave-cli` startup cost pay for the whole run instead of once per block. Measured this session,
directly, on this machine (`octave-cli` 11.1.0):

- **Cold-starting a fresh process per call**: roughly 165–220 ms, *every single call*. Infeasible
  for a transient run with more than a handful of steps.
- **One persistent process for the whole run**: the same real startup cost paid exactly once
  (~158 ms), then roughly **0.044 ms per call** afterward — a genuine three-orders-of-magnitude
  difference once a run has more than a handful of steps, which any real transient does.

`dae-runtime`'s own `simulate_transient_with_blocks` spawns this shared session **lazily** — the
first time it actually encounters a `BlockKind::OctFunc` block while building `block_states`, not
eagerly for every run — so a netlist that never uses `kind=octfunc` at all never pays Octave's
own startup cost. The session lives behind `Rc<RefCell<octave_ffi::OctaveSession>>`, cloned
(cheaply — an `Rc::clone`, never a second `octave-cli` process) into every `BlockState::OctFunction`
instance. This is also why cloning that `BlockState` variant (needed only by
[`TimeStep::Adaptive`]'s retry loop) is always cheap and infallible, unlike `BlockState::CScript`
(which panics without `cscript_clone`) — nothing about a shared, stateless session is corrupted
by a rejected trial step re-running the same call again.

## The call protocol

`octave_ffi::OctaveSession::add_path` sends, once per unique directory containing a `.m` file
this session will call into (Octave requires the file to be on its own path *and* the file name
to match the function name):

```text
addpath('<absolute-dir-containing-the-.m-file>');
```

`octave_ffi::OctaveSession::call` passes every input as a **literal numeric argument**, never
assigned into a named workspace variable:

```text
try
  [__o0, __o1, ...] = <function>(<arg0>, <arg1>, ...);
  printf("%.17g\n", __o0);
  printf("%.17g\n", __o1);
  ...
  printf("@@GS_OCT_<call_id>@@\n");
catch __err
  printf("ERROR: %s\n", __err.message);
  printf("@@GS_OCT_<call_id>@@\n");
end
fflush(stdout);
```

This literal-argument choice is exactly what makes several block instances safely share one
`octave-cli` process: since nothing is ever written to the shared workspace, there is no possible
variable-name collision between two instances calling two different functions on the same
session — confirmed directly this session by `crates/octave-ffi/tests/session.rs`'s own
`two_instances_of_different_functions_share_one_session_without_contamination` test, which
interleaves calls to two genuinely different `.m` functions on one session and checks neither's
result is ever corrupted by the other's.

Stdout is read line by line until the marker line for this call's own `call_id` appears. If the
first line read back is `ERROR: ...`, the call failed on the Octave side — surfaced as
`OctaveError::Runtime`, never misparsed as a numeric output and never a panic. `call_id` is a
per-process-lifetime incrementing counter, embedded in the marker specifically so a desync (a
stray leftover line from some prior malformed exchange) is *detectable* — a mismatched marker or
an unexpected output-line count raises a clear error — rather than silently misread as the next
call's own output.

### Numeric round-tripping

`%.17g` on the Octave side, together with Rust's `str::parse::<f64>`, round-trips a real
fractional double bit-for-bit — confirmed empirically this session against a value with a
genuine precision tail, not a round number: `0.1 + 0.2` (`0.30000000000000004` in both Rust's own
`f64` arithmetic and Octave's) survives the full round trip with identical bits on both ends,
checked directly against `f64::to_bits()`, not just an approximate/tolerance comparison. Rust's
own `f64` `Display` (`format!("{x}")`) already emits the shortest decimal string that round-trips
back to the same bits, which is already valid Octave literal syntax for every finite value; `Inf`/
`-Inf`/`NaN` are special-cased (Octave's own capitalized spelling, which Rust's `Display` does
not produce).

### Error-then-recovery: the session survives a caught Octave-side error

A runtime error caught by the `try`/`catch` (e.g. calling a function with the wrong arity) reports
cleanly via the `ERROR:` line, and **the session survives and keeps working correctly for
subsequent calls** — confirmed empirically this session (`crates/octave-ffi/tests/session.rs`'s
own `runtime_error_is_caught_and_session_survives_for_later_calls`, which deliberately triggers
two independent error-then-recovery cycles on the same session, not just one, to rule out a
lucky ordering). There is no need to respawn the process after a caught Octave-side error, and
this crate never does.

### Process death mid-run must not hang the simulator

If the persistent process dies (crash, killed externally, `octave-cli` itself panics) partway
through a run, a blocking read on its stdout would otherwise hang forever. Every read in
`octave_ffi` detects EOF/a broken pipe on the underlying pipe and returns
`OctaveError::ProcessExited` (carrying any stderr the process produced before dying — captured on
a dedicated background thread specifically so a chatty stderr writer can never deadlock the pipe
against this crate's own synchronous stdout reads) instead of hanging.

This is exercised by actually killing the child process, not just by reviewing the read/EOF-
handling code by eye:
`crates/octave-ffi/tests/session.rs::process_dies_mid_run_reports_clean_error_instead_of_hanging`
confirms the call, kills the process, then confirms the *next* call returns
`OctaveError::ProcessExited` promptly (bounded by a background-thread timeout, so a genuine
regression that reintroduces a hang fails the test loudly instead of blocking the whole suite).

## Why `--interactive` is deliberately not used

The obvious way to keep `octave-cli` reading commands forever, `--interactive` (`-i`), turned out
to be actively wrong for this protocol: confirmed empirically this session, with `--interactive`
Octave prints its own `octave:N>` prompt to stdout before every statement, which corrupts this
crate's own line-oriented marker protocol (a prompt line landing where an output line or the
marker itself was expected). Also confirmed empirically: plain `octave-cli` (no `--interactive`
at all), given a genuine OS pipe as stdin (never a tty), already reads and executes each
statement as it arrives without ever waiting for stdin to close — exactly the persistent-session
behavior this crate needs, with none of `--interactive`'s prompt-printing side effect. There is
no flag that keeps `--interactive`'s "never exit at EOF" behavior without also printing prompts,
so `--interactive` is simply not passed at all; `octave_ffi::OctaveSession::spawn` uses
`--no-gui --norc --quiet` only.

## What's deliberately not built (`kind=octfunc`)

- No `state`/`t`/`dt`/`xc_count` at all — genuinely out of scope for a stateless function, the
  same restriction `kind=pyfunc` already has, for the same reason. A netlist author who needs
  persistent state should reach for `kind=octblock` instead (see below).
- No vector-input or vector-output support — every input must be `SignalValue::Scalar` (checked
  via `dae-runtime`'s own `require_all_scalar` helper, the same one several other scalar-only
  block kinds already use), matching the literal-numeric-argument protocol above (there is no
  natural Octave-literal spelling for an arbitrary vector argument that would be worth the added
  protocol complexity) and matching `kind=pyfunc`'s own scalar-only *output* restriction.
- `ts=variable` is rejected at parse time, with the same reasoning `kind=pyfunc` already uses (a
  stateless function call has no instance to remember a requested next-hit time against) — see
  `general-mna::system_builder::parse_sample_time`'s own `kind_str`-parameterized rejection
  message, shared by both `pyfunc` and `octfunc`.

## Stateful blocks (`kind=octblock`)

`kind=octblock` is the `pyblock` analog: a user-supplied Octave-compatible function with real
persistent state across steps (`start`/`output`/`update`/`xc_count`, the same contract shape
`pyblock`/`cscript` already have), on top of the exact same `octave_ffi::OctaveSession` and
shared-persistent-process design above — no second `octave-cli` process, no separate crate. Two
constraints, both discovered empirically against the real installed `octave-cli` (11.1.0), drove
this design; neither was assumed going in.

### Constraint 1: Octave only auto-loads the first function in a `.m` file

Confirmed directly: given a `.m` file with two `function ... end` blocks, only the *first* one
(matching the file's own name) is callable from outside the file — the second is an invisible
private subfunction, exactly Octave's own documented scoping rule for a `.m` file, not a bug or
a version quirk. `pyblock`'s own single `.py` file holding `start`/`output`/`update`
all as top-level `def`s has no such restriction (Python has no "only the first def counts"
rule), so this is a genuine, Octave-specific wrinkle `pyblock`'s own contract shape can't be
copied verbatim.

**Consequence: every contract function needs its own file**, named `<function>_<role>.m`,
sharing one directory (`kind=octblock`'s own `path=`, which therefore names a *directory*, not
a single file — the one place this contract's own `path=` semantics diverge from every other
block kind's):

| File | Required | Signature |
|---|---|---|
| `<function>_start.m` | always | `state = <function>_start()` |
| `<function>.m` | unless `xc_count>0` | `[new_state, y1, y2, ...] = <function>(state, t, dt, u1, u2, ...)` |
| `<function>_derivative.m` | only if `xc_count>0` | `xc_dot = <function>_derivative(state, u1, ..., xc)` — pure, must not mutate `state`; called up to 4×/step (RK4 stages) |
| `<function>_output_xc.m` | only if `xc_count>0` | `[new_state, y1, ...] = <function>_output_xc(state, t, dt, u1, ..., xc)` — `xc` here is this step's own already-integrated continuous state |
| `<function>_update.m` | optional | `new_state = <function>_update(state, t, dt, u1, ...)` — called once per resolved step, after output; if absent, `output`/`output_xc`'s own returned `new_state` is the only state advance |
| `<function>_next_sample_hit.m` | only if `ts=variable` | `seconds = <function>_next_sample_hit(state, t, dt, u1, ...)` |

This is documented prominently — in the component reference's own `## Description` (not buried
in a caveat) — specifically because a user coming from Octave/legacy-tooling habits of "one
script, many local functions" hits this on their very first attempt.

`dae_runtime::DaeError::OctBlockMissingRequiredFile`/
`OctBlockRequiresNextSampleHitForVariableSampleTime` check every required file's existence
directly against the filesystem, once, at construction — *before* `octave-cli` is ever asked
about it, so a missing file is a clear, immediate error naming the exact expected path, not a
generic Octave-side "undefined function" surfacing at the first call.

### Constraint 2: per-instance state lives inside the Octave session itself, never serialized back

Unlike `cscript`'s opaque `void*`/`pyblock`'s opaque Python object handle — both owned by a
Rust-side struct — an `octblock` instance's own `state` (whatever `<function>_start` returns)
lives entirely **inside the shared `octave-cli` session**, in one global struct keyed by the
block's own instance name:

```text
global __gs_state;
__gs_state.('<instance>') = <function>_start();     % once, at construction

% each resolved step:
global __gs_state;
try
  [__gs_state.('<instance>'), __o0, ...] = \
      <function>(__gs_state.('<instance>'), <t>, <dt>, <u0>, ...);
  printf("%.17g\n", __o0); ...
  printf("@@GS_OCT_<call_id>@@\n");
catch __err
  printf("ERROR: %s\n", __err.message);
  printf("@@GS_OCT_<call_id>@@\n");
end
```

Rust's own job is just "call `output` for instance `A`" — `dae_runtime::BlockState::OctBlock`
holds only `instance: String` (the lookup key) plus the same zero-order-hold bookkeeping every
other sample-time-gated block already carries; it never holds a copy of the opaque state itself.
This was validated directly against real `octave-cli`
(`crates/octave-ffi/tests/stateful_session.rs`'s own
`two_instances_of_same_stateful_function_do_not_contaminate_each_others_state`): two independent
instances of the *same* Octave function, calls interleaved both orderings, zero
cross-contamination — purely a consequence of `instance` being a distinct struct field name in
one shared global.

**A failed call cannot corrupt state.** Octave never performs a multi-value assignment's
left-hand-side writes if evaluating the right-hand side raises — so `__gs_state.('<instance>')`
is only overwritten on a call that actually succeeds; a caught runtime error leaves the previous
state slot completely untouched, with no special-casing needed on this crate's own side to get
that guarantee. Confirmed directly:
`crates/octave-ffi/tests/stateful_session.rs::failed_stateful_call_does_not_corrupt_instance_state`
deliberately triggers two independent error-then-recovery cycles and checks the instance's own
state slot is exactly what it was before each failed call, not silently reset or clobbered.

**The one exception: `xc` crosses the pipe explicitly.** The solver-owned continuous-state
vector (the `xc_count > 0` case) is *not* part of the block's own opaque state — it's the
solver's own state, integrated by `dae-runtime`'s RK4 stepper the same way `cscript`'s/
`pyblock`'s own `xc` is — so it has to cross the pipe as explicit numeric data on every call,
formatted as a literal Octave row-vector argument (`[v0, v1, ...]`), the one place this protocol
passes anything beyond scalar literals. `octave_ffi::OctaveSession::rk4_step_xc_stateful` mirrors
`cscript_ffi::CScriptInstance::rk4_step_xc` exactly: RK4 done in Rust, calling
`<function>_derivative` up to four times per step, holding the block's other inputs fixed across
all four stages.

### `octave_ffi::OctaveSession`'s own stateful API

Four methods, added alongside `octfunc`'s existing `call`/`add_path` (left byte-for-byte
unchanged):

- `call_start(function, instance)` — `__gs_state.('<instance>') = <function>_start()`.
- `call_stateful(function, instance, args, xc, num_outputs)` — the state-*writing* shape
  (`output`/`output_xc`/`update` all share it — `update` just passes `num_outputs = 0`).
- `call_readonly_stateful(function, instance, args, xc, num_outputs)` — the state-*reading*
  shape (`derivative`/`next_sample_hit`), where `__gs_state.('<instance>')` is passed in as an
  argument but never reassigned.
- `rk4_step_xc_stateful(derivative_function, instance, xc, args, dt)` — the RK4 stepper above,
  built on `call_readonly_stateful`.

### A real, documented limitation: no `TimeStep::Adaptive` support

`TimeStep::Adaptive`'s own retry loop clones the whole `block_states` vector before every trial
step, and simply discards a rejected trial's clone — the protocol every other stateful block kind
(`cscript`, `pyblock`) depends on: their own state lives *inside* the cloned Rust value, so a
discarded trial's mutations are discarded along with it. `octblock`'s own state lives inside the
one shared `octave-cli` session, which is **never cloned** — a rejected trial's own
`output`/`output_xc`/`update` calls would have already mutated the real Octave-side state, with
no way to roll them back once the trial is discarded and retried with a smaller `dt`.

Rather than silently producing wrong answers under adaptive stepping, `kind=octblock` is
rejected outright at construction time whenever `TimeStep::Adaptive` is requested —
`dae_runtime::DaeError::OctBlockDoesNotSupportAdaptiveStep`, unconditional (unlike
`CScriptRequiresCloneForAdaptiveStep`, which is opt-in and satisfiable by exporting
`cscript_clone` — there is no `.m`-file convention that could make Octave's own shared
global-struct state trial-cloneable). A netlist using `kind=octblock` must run with a fixed
`--dt`.
