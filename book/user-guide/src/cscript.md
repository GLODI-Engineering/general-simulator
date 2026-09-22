# The CScript escape hatch

`kind=cscript` loads a user-supplied, precompiled native shared library (`.so`/`.dylib`/`.dll`)
and calls into it once per step. It exists for genuinely stateful block behavior that none of
`continuous-blocks`'s own blocks cover — an integrator with a custom nonlinearity, a lookup table
built once at startup from a file, a bit-exact port of an existing embedded control algorithm.
Reach for it only once you've checked the [Component Reference](component-reference.md) and
confirmed nothing there already does what you need; composing `pid`/`statespace`/`tf`/the
waveform-arithmetic blocks is almost always simpler to get right and doesn't require compiling
anything.

**This is the one place in the whole workspace where arbitrary native code loads into the
process.** Calling into a `.so` you (or someone else) compiled is unsafe by construction —
in-process, full host privileges, capable of corrupting memory or crashing the process outright.
This is an accepted, bounded risk of an explicitly opt-in feature (you name your own `.so` in
your own netlist), not something sandboxed away — treat a `kind=cscript lib=` line the same way
you'd treat running any other native binary you didn't audit yourself.

## The C-side contract, summarized

Full detail (including every edge case) lives in `crates/cscript-ffi/src/lib.rs`'s own module
doc comment — this section is the "what do I need to write" summary a netlist author needs, not
the implementation-level contract.

**Required, always:**

```c
void *cscript_start(void);
```

Called once per block instance, returning whatever opaque state pointer your library wants to
keep around (or `NULL`/`0` if you have none — see the example below).

**The plain, single-function contract** (the common case — `xc_count=0`, the default):

```c
void cscript_output(void *state, const double *in, int in_len, double dt,
                     double *out, int out_len);
```

Called once per resolved step (or once per `ts=`/`freq=` sample hit — see below), reading
`in`/`in_len` inputs and writing `out`/`out_len` outputs. `dt` is that step's own resolved size.

**Optional exports:**

- `void cscript_free(void *state);` — called once when the block is torn down. If absent, the
  state pointer is simply leaked — harmless for a short-lived CLI run, but real for anything
  long-running.
- `void *cscript_clone(void *state);` — see "Adaptive step-size control" below. **Not optional**
  if you want to run under adaptive stepping.
- `size_t cscript_state_size(const void *state);`, `void cscript_state_write(const void *state,
  uint8_t *out);`, `void *cscript_state_read(const uint8_t *in, size_t len);` — all three
  together, see "Checkpoint and resume" below. **Not optional** if you want to run under
  `--checkpoint-out`/`--resume`.

## A complete minimal example

`doc-verify/cscript/gain.c` — a fixed ×2 gain, no internal state at all:

```c
void *cscript_start(void) {
    return (void *)0;
}

void cscript_output(void *state, const double *in, int in_len, double dt, double *out, int out_len) {
    (void)state;
    (void)dt;
    if (in_len >= 1 && out_len >= 1) {
        out[0] = in[0] * 2.0;
    }
}
```

Compiled with `cc -shared -fPIC -O0 -o gain.so gain.c` and wired in from `doc-verify/cscript/example.cir`:

```text
SRC kind=const value=3
G1 kind=cscript lib=doc-verify/cscript/gain.so in=SRC
```

Verified end to end: `G1` tracks $2 \times \text{SRC}$ every step — $\text{SRC}=3$ produces
$G_1=6$. Start from this
fixture (or `doc-verify/cscript/gain.c`/`example.cir` directly) rather than writing a `.c` file
from scratch.

## `outputs=` and `ts=`/`freq=`

- `outputs=<name1,name2,...>` — this block's own output names; defaults to a single output
  aliasing the block's own `.name` if omitted. Use this when `cscript_output` fills more than
  one `out[]` slot.
- `ts=<f64>` / `freq=<f64>` (mutually exclusive with each other, and with `ts=variable`) give
  this block its own fixed sample period, independent of the circuit's own resolved step size.
  `cscript_output` only actually runs once accumulated time reaches that period; between hits,
  the block holds its last output (zero-order hold) — the right model for a genuinely discrete
  controller running at a fixed rate (a digital control loop clocked well below the switching
  frequency), as opposed to something meant to behave continuously. Omit both to run
  `cscript_output` every resolved circuit step instead.
- `ts=variable` — the block computes its own schedule by exporting
  `double cscript_next_sample_hit(void *state, ...)`, returning seconds (relative to the call)
  until the block should run again. Exclusive with `freq=`/`to=`, since there's no fixed
  period/offset to combine a self-computed schedule with.

A path containing whitespace must be `"double-quoted"` (`lib="my libs/gain.so"`) — every field is
whitespace-tokenized like any other, and this is the one place a path is likely to contain a
space.

## The optional continuous-state (`xc`) contract

If your block's state is more naturally expressed as ODE state the *solver* should integrate
(rather than something you hand-integrate yourself inside `cscript_output`), set
`xc_count=<usize>` to the number of continuous states, and export a different pair of functions
*instead of* `cscript_output`:

```c
void cscript_derivative(void *state, const double *in, int in_len,
                         const double *xc, int xc_len, double *xc_dot, int xc_dot_len);
void cscript_output_xc(void *state, const double *in, int in_len, double dt,
                        const double *xc, int xc_len, double *out, int out_len);
```

`cscript_derivative` is a pure vector field — called up to four times per step (RK4's own `k1`
through `k4`), and must not mutate `state`/`in`/`xc`. `cscript_output_xc` then produces this
step's actual output from the now-integrated `xc`. `xc_count=0` (the default) is the plain,
single-function contract above, unchanged; a nonzero `xc_count` requires the derivative/output_xc
pair instead. `cscript_start`/`cscript_free`/`cscript_clone` are unaffected either way.

There's also an optional `cscript_update(void *state, const double *in, int in_len, double dt,
double *xc, int xc_len)`, called immediately after `cscript_output`/`cscript_output_xc` on every
step the block actually runs — a dedicated place to advance a *discrete* piece of state (a
counter, a fixed-rate filter's own delay line) separately from the continuous `xc` the solver
integrates, if you'd rather not fold it into `cscript_output` directly.

This is a deep part of the contract — see `cscript_ffi`'s own module doc comment and the dev
guide for the full implementation-level detail (memory-safety obligations across the FFI
boundary, exact call ordering, what happens on a rejected adaptive trial). This chapter only
covers enough to know the `xc` path exists and when to reach for it.

## Adaptive step-size control requires `cscript_clone`

**Call this out prominently, because it's the error you'll actually hit**: adaptive stepping
(the default whenever `--dt` is omitted — see [Fixed vs. adaptive time
stepping](time-stepping.md)) clones every block's state before each trial step and discards the
clone if the trial is rejected. An opaque C state pointer can't be deep-copied without the
library's own help, so a `kind=cscript` block whose library doesn't export `cscript_clone` fails
immediately under adaptive stepping, at the very first step, with:

```text
CScriptRequiresCloneForAdaptiveStep { block_name: "<name>" }
```

(this is the CLI's raw `Debug` formatting of `dae_runtime`'s own error type, not a hand-written
message — searchable verbatim). `doc-verify/cscript/error_adaptive_needs_clone.cir` reproduces
it directly against `gain.c` above, which deliberately exports no `cscript_clone`. The fix is
one of two things: export `cscript_clone` from your library, or run with a fixed `--dt` instead.

## Checkpoint and resume

[Checkpoint and resume](checkpoint-resume.md) writes every block's state to a file and reads it
back later, possibly in another process. Your `void *state` is the one thing the simulator
cannot serialize on its own — only your code knows its layout — so a library opts in by
exporting **all three** of:

```c
#include <stdint.h>
#include <stddef.h>

// How many bytes cscript_state_write() will produce for `state`. `state` is whatever
// cscript_start()/cscript_clone()/cscript_state_read() returned -- NULL for a stateless
// block, in which case return 0.
size_t cscript_state_size(const void *state);

// Serialize `state` into `out`, which has exactly cscript_state_size(state) bytes of room.
// Never called when that size is 0.
void cscript_state_write(const void *state, uint8_t *out);

// The inverse: build a fresh, independently owned state from `len` bytes written by
// cscript_state_write() -- possibly by another process -- and return it. The instance's
// previous state is released through cscript_free() (if you export one) afterwards.
void *cscript_state_read(const uint8_t *in, size_t len);
```

For a plain-old-data struct, each half is a `memcpy`. `doc-verify/cscript/accumulator_checkpoint.c`
is the complete worked example — a running sum with all three functions — and
`doc-verify/cscript/checkpoint_example.cir` runs it split across a checkpoint and asserts the
CSV is byte for byte the uninterrupted run's. Write every value the block needs to continue
*bit-identically*; a struct holding pointers must serialize what they point at, not the
addresses. `xc` needs nothing from you: the solver owns it and saves it itself.

The three are **all-or-nothing**:

- Export **none** of them and nothing changes: the block runs exactly as before without
  `--checkpoint-out`/`--resume`, and a run that asks for a checkpoint is refused at the first
  one with `CheckpointUnsupportedBlock { block: "<name>", kind: "cscript" }` — never written
  partially, no file left behind (`doc-verify/cscript/error_checkpoint_unsupported.cir`).
- Export **some** of them and the library does not load at all, checkpoint or not, so a
  half-adopted contract cannot masquerade as a library that never opted in:

  ```text
  error: CScript(MissingSymbol { path: "doc-verify/cscript/partial_state.so", symbol: "cscript_state_read", message: "the checkpoint state contract is all-or-nothing: cscript_state_size, cscript_state_write and cscript_state_read must all be exported, or none of them" })
  ```

  (`doc-verify/cscript/error_partial_state_contract.cir`, against `partial_state.c`, which
  exports only the first two.)

## What isn't checked

A `.so` missing a required export, or a signature mismatch, surfaces as a load-time or call-time
failure from `cscript_ffi` itself rather than a netlist-parse-time error — there's no way to
validate a shared library's actual exported signatures before trying to call them. Get the
minimal example above working first, then extend it, rather than debugging a hand-written `.c`
file and the netlist wiring simultaneously.
