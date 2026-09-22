# The Octave escape hatch

`kind=octfunc` and `kind=octblock` call into a user-supplied, Octave-compatible `.m` file —
the Octave-hosted counterpart to [the Python escape hatch](python-blocks.md), for people with
legacy `.m`-format scripts instead of Python. As with `cscript`/`pyblock`, check
the [Component Reference](component-reference.md) first — composing existing blocks is almost
always simpler than writing and debugging `.m` files against this contract.

Two block kinds, one chapter, mirroring the Python chapter's own split:

- **`kind=octfunc`** — a plain, stateless function, called with each input as its own
  positional argument. No persistent state, no `t`/`dt`.
- **`kind=octblock`** — a stateful block with the same `start`/`output`/`update`/`xc_count`
  shape `pyblock`/`cscript` have. **Does not support adaptive step-size control** — see below,
  this is a real, hard limitation, not a caveat to skim past.

## This needs a real `octave-cli` binary — not a Cargo feature

**Unlike `kind=pyblock`/`kind=pyfunc`, this is never feature-gated at build time.** GNU Octave
is GPLv3-licensed; linking its embedding library (`liboctinterp`) into this project's own
permissively-licensed binaries would risk pulling them under GPL. So `crates/octave-ffi` never
links against any Octave C/C++ library and has no Octave crate/library as a build-time
dependency at all — per that crate's own `src/lib.rs` module doc comment, the only integration
is spawning the separately installed `octave-cli` *binary* as a subprocess ("mere aggregation"
under GPL, not linking, the same reasoning that lets tools shell out to `ffmpeg`/`gs` without
inheriting their license). The practical upshot:

```bash
cargo build --release -p general-simulator-cli   # no --features flag needed for kind=octfunc/octblock
```

builds unconditionally on every platform, and only fails at *runtime*, with a clear
`octave_ffi::OctaveError::NotFound`, if `octave-cli` isn't on `PATH` when a `kind=octfunc`/
`kind=octblock` block is actually evaluated. You do need `octave-cli` genuinely installed and on
`PATH` to *run* a netlist that uses either block kind — [Installing and
building](getting-started.md) notes the same requirement for `octave-ffi`'s own test suite.

Every `kind=octfunc`/`kind=octblock` instance in one simulation run shares a single, lazily
spawned, persistent `octave-cli` process (never a separate process per block instance) —
measured on the machine this chapter's examples were verified on (`octave-cli` 11.1.0): roughly
158 ms to start that one process, then roughly 0.044 ms per call afterward, against
165–220 ms *per call* if a fresh process were spawned every time. A netlist that never uses
either block kind never pays that startup cost at all.

## `kind=octfunc`: stateless functions

### The contract

An `.m` file whose name (minus `.m`) must equal the function it defines — Octave's own
requirement:

```octave
function y = add_one(x)
  y = x + 1;
end
```

Called as `f(arg0, arg1, ...)` — each declared `inputs=` entry (or the single `in=` signal) is
its own positional argument, never bundled into one array. Returns a single value, or as many
comma-separated return values as `outputs=` declares (`[y1, y2] = f(...)`).

### A complete minimal example

`doc-verify/octfunc/add_one.m`:

```octave
function y = add_one(x)
  y = x + 1;
end
```

Wired in from `doc-verify/octfunc/example.cir`:

```text
SRC kind=const value=3
G1 kind=octfunc path=doc-verify/octfunc/add_one.m function=add_one in=SRC
```

Verified end to end against a real `octave-cli` (11.1.0) and a plain `cargo build --release
-p general-simulator-cli` — real CLI stdout, `SRC=3` giving `G1=4` at every step:

```text
t,SRC,G1
0.00001,3,4
0.00002,3,4
0.000030000000000000004,3,4
...
0.0001,3,4
```

`inputs=A,B` calling `f(A, B)` as two genuinely distinct positional arguments is exercised
separately by `doc-verify/octfunc/example_two_inputs.cir`.

### Parameters

- `path=<file.m>` — the `.m` file containing `function`; its own file name (minus `.m`) must
  equal `function`. `"double-quote"` a path containing whitespace, same as `cscript`'s `lib=`.
- `function=<name>` — required, no default.
- `outputs=<name1,name2,...>` — output names; defaults to a single output aliasing the block's
  own `.name`.
- `in=<signal>` or `inputs=<sig1,sig2,...>` — exactly one of these.
- `ts=<f64>` / `freq=<f64>` [`to=<f64>`] — optional fixed sample period, same zero-order-hold
  convention as `cscript`'s own. `ts=variable` is **rejected at parse time**, same reasoning as
  `kind=pyfunc`:

  ```text
  ts=variable is not available for kind=octfunc (a stateless function call has no instance to
  remember a requested next-hit time against)
  ```

There is no `xc_count=` here either, for the same reason `kind=pyfunc` has none.

Every input must be a scalar (`SignalValue::Scalar`) — there is no vector-input or vector-output
support for `kind=octfunc`, since there's no natural Octave-literal spelling for an arbitrary
vector argument worth the added protocol complexity (see "The call protocol" below).

## `kind=octblock`: stateful blocks — `path=` names a directory, not a file

### The Octave-side contract, summarized

This is the one place a `kind=octblock` contract genuinely diverges from `pyblock`'s own shape,
for a real Octave-specific reason: **Octave only auto-loads the *first* `function ... end` block
in a `.m` file** (confirmed directly against real `octave-cli` — a second function in the same
file is an invisible private subfunction, not a bug or version quirk). A single `.py` file
holding `start`/`output`/`update` as top-level `def`s has no such restriction, so that pattern
can't be copied verbatim into Octave.

**Consequence: every contract function needs its own file**, named `<function>_<role>.m`, all
living together in one directory — and `path=` for `kind=octblock` names *that directory*, not
a single `.m` file:

| File | Required | Signature |
|---|---|---|
| `<function>_start.m` | always | `state = <function>_start()` |
| `<function>.m` | unless `xc_count>0` | `[new_state, y1, y2, ...] = <function>(state, t, dt, u1, u2, ...)` |
| `<function>_derivative.m` | only if `xc_count>0` | `xc_dot = <function>_derivative(state, u1, ..., xc)` — pure, must not mutate `state`; called up to 4×/step (RK4 stages) |
| `<function>_output_xc.m` | only if `xc_count>0` | `[new_state, y1, ...] = <function>_output_xc(state, t, dt, u1, ..., xc)` |
| `<function>_update.m` | optional | `new_state = <function>_update(state, t, dt, u1, ...)` — called once per resolved step, after output; if absent, `output`/`output_xc`'s own returned `new_state` is the only state advance |
| `<function>_next_sample_hit.m` | only if `ts=variable` | `seconds = <function>_next_sample_hit(state, t, dt, u1, ...)` |

A user coming from "one script, many local functions" habits hits this on their very first
attempt — it is the load-bearing difference from every other block kind's own `path=`, not a
buried caveat.

A required `<function>_*.m` file missing from `path`'s own directory is checked once, up front,
at construction time, *before* `octave-cli` is ever asked about it:

```text
dae_runtime::DaeError::OctBlockMissingRequiredFile { block_name, expected_path }
```

### A complete minimal example

A stateful accumulator (`out = state + in`, state advances by `in` every step) —
`doc-verify/octblock/accum/accumulate_start.m`:

```octave
function state = accumulate_start()
  state = 0;
end
```

and `doc-verify/octblock/accum/accumulate.m`:

```octave
function [new_state, y] = accumulate(state, t, dt, u)
  new_state = state + u;
  y = new_state;
end
```

Wired in from `doc-verify/octblock/example.cir`:

```text
SRC kind=const value=3
G1 kind=octblock path=doc-verify/octblock/accum function=accumulate in=SRC
```

Verified end to end against real `octave-cli` — real CLI stdout at `--tfinal 4e-5 --dt 1e-5`,
`G1` carrying state forward across four steps ($3, 6, 9, 12$, matching `SRC=3` accumulated once
per step):

```text
t,SRC,G1
0.00001,3,3
0.00002,3,6
0.000030000000000000004,3,9
0.00004,3,12
```

Each instance's own state (whatever `<function>_start` returns) lives entirely **inside the
shared `octave-cli` session itself**, in one global struct keyed by the block's own instance
name — it is never handed back to Rust step by step the way `pyblock`'s opaque Python-object
handle or `cscript`'s opaque `void*` are (the one exception is a checkpoint, see below). Two instances of the *same* Octave function never contaminate
each other's state (validated directly against real `octave-cli`, interleaved calls, zero
cross-contamination). A failed call — an Octave-side exception caught by this crate's own
`try`/`catch` protocol — cannot corrupt that state either: Octave never performs a multi-value
assignment's left-hand-side writes if evaluating the right-hand side raises, so a caught runtime
error leaves the previous state slot completely untouched.

### `outputs=`, `ts=`/`freq=`, and the `xc_count` contract

Mean exactly the same as `kind=pyblock`'s own equivalents — see [that chapter's
sections](python-blocks.md#outputs-and-tsfreq) for the zero-order-hold convention and the
solver-integrated-vs-hand-managed state split. `ts=variable` here requires
`<function>_next_sample_hit.m` to also exist in `path`'s own directory; if it's declared without
that file present:

```text
dae_runtime::DaeError::OctBlockRequiresNextSampleHitForVariableSampleTime
```

The `xc_count>0` contract (`_derivative.m`/`_output_xc.m`) is verified end to end against its
own analytic solution the same way `pyblock`'s is: a first-order charge, $\dot{x}_c = 1 - x_c$,
starting at rest, so $x_c(t) = 1 - e^{-t}$ — see `doc-verify/octblock/xc_example.cir`. The one
piece of state that *does* cross the Octave-session boundary as explicit numeric data (rather
than living inside the session's own global struct) is this solver-owned `xc` vector itself,
formatted as a literal Octave row-vector argument on every call — that's the *solver's* state,
not the block's own opaque one, exactly mirroring `cscript`'s/`pyblock`'s own `xc` split.

### Parameters

- `path=<dir>` — the directory containing `<function>_*.m` (see the file table above).
  `"double-quote"` a path containing whitespace.
- `function=<name>` — required, no default; the base name every `<function>_<role>.m` file
  shares.
- `outputs=<name1,name2,...>` — output names; defaults to a single output aliasing the block's
  own `.name`.
- `in=<signal>` or `inputs=<sig1,sig2,...>` — exactly one of these.
- `ts=<f64>` / `freq=<f64>` [`to=<f64>`] / `ts=variable` — optional.
- `xc_count=<usize>` — optional, default `0`.

### Checkpoint and resume

[Checkpoint and resume](checkpoint-resume.md) works for `kind=octblock` with **no change to
your `.m` files**. At a checkpoint the simulator copies the instance's own state slot to a
scratch variable and writes it with Octave's own `save -binary` to a short-lived temporary
file, reads the bytes into the checkpoint, and deletes the file; on `--resume` it writes the
bytes back out, `load`s them, and assigns the result into the same slot, replacing what
`<function>_start` had just put there. Whatever your `_start` returned — a scalar, a struct, a
matrix — goes along, and Octave's binary format stores doubles as their exact bits, so the
resumed run is bit-identical to an uninterrupted one exactly as it is for the built-in blocks.
`doc-verify/octblock/checkpoint_example.cir` runs the unmodified accumulator above split across
a checkpoint and asserts the CSV is byte for byte the uninterrupted run's.

## Adaptive step-size control is not supported — a hard limitation, not a caveat

**Run every `kind=octblock` netlist with a fixed `--dt`.** [Adaptive
stepping](time-stepping.md) clones every block's state before each trial step and discards the
clone if the trial is rejected — the protocol `cscript`/`pyblock` both rely on, since their own
state lives *inside* the cloned Rust value. `kind=octblock`'s state lives inside the one shared
`octave-cli` session instead, which is **never cloned** — there is no `.m`-file convention that
could make Octave's own shared global-struct state trial-cloneable the way `cscript_clone`/
Python's `deepcopy` make the other two block kinds' state cloneable. A rejected adaptive trial's
own `output`/`output_xc`/`update` calls would already have mutated the real Octave-side state,
with no way to roll them back once the trial is discarded and retried with a smaller step.

Rather than silently producing wrong answers, `kind=octblock` is rejected outright at
construction time whenever adaptive stepping is requested (i.e. `--dt` is omitted). Confirmed
directly against this build, running `doc-verify/octblock/error_adaptive_not_supported.cir`
(the accumulator example above, with no `--dt` flag) fails immediately with:

```text
error: OctBlockDoesNotSupportAdaptiveStep { block_name: "G1" }
```

Unlike `CScriptRequiresCloneForAdaptiveStep` (which is opt-in and satisfiable by exporting
`cscript_clone`), this rejection is unconditional — there is no author-side fix beyond running
with a fixed `--dt`. `kind=octfunc` has no equivalent problem: since it's stateless and shares
one process with no per-instance state to isolate, cloning it is always trivial and it runs
correctly under adaptive stepping.

## The call protocol, briefly

Every input crosses into Octave as a **literal numeric argument**, never assigned into a named
workspace variable — this is what makes several block instances safely share one `octave-cli`
process with no risk of variable-name collision between two instances calling two different
functions. Numeric values round-trip through `%.17g` formatting on the Octave side and Rust's
`str::parse::<f64>`, confirmed to preserve the value bit-for-bit even for a genuine
precision-tail value ($0.1 + 0.2$). See `crates/octave-ffi/src/lib.rs`'s own module doc comment
and `book/dev-guide/src/octave-blocks.md` for the full protocol (the marker-line desync
detection, error-then-recovery behavior, and what happens if the shared process dies mid-run).

## What isn't checked

An Octave-side exception raised by a called function, or a malformed/desynchronized reply from
the shared session, surfaces as a call-time failure from `octave_ffi` itself
(`dae_runtime::DaeError::Octave`), not at netlist-parse time — there's no way to validate an
`.m` file's actual behavior before calling into it. For `kind=octblock` specifically, a missing
required file *is* caught early (see above); a wrong function signature inside a file that does
exist is not, and surfaces as an Octave-side error at the first call that reaches it. Get the
minimal example above working first, then extend it, rather than debugging a hand-written `.m`
file and the netlist wiring simultaneously.
