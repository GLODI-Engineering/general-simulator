# Octave pure-function blocks (`kind=octfunc`): design

*(Implemented this session, as a genuinely separate, additive block kind alongside `pyfunc` —
the direct `octave-cli`-hosted analog, not a mode of anything else. Mirrors `pyfunc-blocks.md`'s
own before/after structure.)*

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

## What's deliberately not built

- No `state`/`t`/`dt`/`xc_count` at all — genuinely out of scope for a stateless function, the
  same restriction `kind=pyfunc` already has, for the same reason. A netlist author who needs
  persistent state should reach for `kind=pyblock` instead (there is no Octave-hosted
  counterpart to `pyblock` in this pass — scope was deliberately kept to the stateless analog
  only).
- No vector-input or vector-output support — every input must be `SignalValue::Scalar` (checked
  via `dae-runtime`'s own `require_all_scalar` helper, the same one several other scalar-only
  block kinds already use), matching the literal-numeric-argument protocol above (there is no
  natural Octave-literal spelling for an arbitrary vector argument that would be worth the added
  protocol complexity) and matching `kind=pyfunc`'s own scalar-only *output* restriction.
- `ts=variable` is rejected at parse time, with the same reasoning `kind=pyfunc` already uses (a
  stateless function call has no instance to remember a requested next-hit time against) — see
  `general-mna::system_builder::parse_sample_time`'s own `kind_str`-parameterized rejection
  message, shared by both `pyfunc` and `octfunc`.
