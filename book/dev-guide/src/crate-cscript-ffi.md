# `cscript-ffi`: the native escape hatch

Loads a user-supplied, precompiled shared library (`.so`/`.dylib`/`.dll`) and calls into it
once per block instance per transient step — a dynamically-loaded, stateful escape hatch for
block behavior no existing `continuous-blocks` block covers. The role it plays is the same
one a code-generation/scripting slot plays in other block-diagram tools' extensibility
mechanisms, or an ngspice XSPICE codemodel `.cm` plugin (an internal gotcha note on the
ngspice XSPICE codemodel needing `MFBINIT` set in its environment records
that already-used pattern).

## Why this exists despite "Rust throughout" — and the scope boundary

`AGENTS.md`'s "Numeric stack" section says "Rust throughout; no C/C++ FFI." This crate is the
one deliberate exception, and `crates/cscript-ffi/src/lib.rs`'s module doc states the
boundary in both directions. First, why the exception is legitimate: genuinely *stateful*
block behavior — an integrator, a lookup table built at startup, anything — cannot be
expressed by `continuous-blocks`' own types, and rather than grow a bespoke Rust block for
every such case, the project exposes one escape hatch and stops there. Second, what the
exception costs, stated plainly and without euphemism:

> **This crate is the one place in this workspace where calling into arbitrary native code is
> deliberately allowed.** Loading a shared library and calling into it is unsafe by
> construction: the code runs in-process with this program's own privileges, can corrupt
> memory, crash the process, or do anything a normal native program can do. That is an
> accepted, bounded risk of this specific opt-in feature (a user explicitly names a `.so`
> file in their own netlist), not something this crate tries to sandbox away.

One place, opt-in, user-named. Not a crack in the rule — the rule's own declared boundary.
The user-facing summary of the same contract (fields, `outputs=`, `ts=`/`freq=`) is
`book/user-guide/src/cscript.md`; the netlist-facing reference entry is the Component
Reference's CScript entry. This chapter is the C-side contract itself.

## The C-side contract

A block's shared library must export (signatures verbatim from `lib.rs`'s module doc):

```c
void *cscript_start(void);
void cscript_output(void *state, const double *in, int in_len, double dt,
                    double *out, int out_len);
```

- `cscript_start` — called once, when the block instance is created. Returns an opaque
  instance-state pointer (typically malloc'd), or NULL for a stateless block.
- `cscript_output` — called once per resolved step. `state` is whatever `cscript_start`
  returned (NULL if none); `in`/`in_len` is this step's input vector in the netlist's own
  declared order; `dt` is this step's size in seconds (needed for any block that integrates
  over time — `state->integral += error * dt`, the same explicit-Euler update every other
  stateful block in the project already uses); `out`/`out_len` is the output vector to fill.

Two optional functions extend it:

```c
void cscript_free(void *state);
void *cscript_clone(void *state);
```

- `cscript_free` — called once when the instance drops. Absent symbol ⇒ `state` simply leaks,
  "harmless for a short-lived CLI run, but real cleanup is provided for any longer-lived
  host."
- `cscript_clone` — returns a **deep copy** of `state` (independent memory: mutating the copy
  through future calls must never affect the original, and vice versa). Only needed under
  adaptive stepping — see below.

Ownership is entirely the library author's: the crate holds an opaque pointer it never
interprets, dereferences, or frees except through these calls. What *is* checked at load
time — symbol presence, and which contract a library implements (plain `cscript_output`, or
the `xc` pair below, never both) — surfaces as a load-time or call-time failure from
`cscript_ffi`'s own error type; what is *not* checked is everything the C code does with the
pointers once called, by construction.

### The optional continuous-state (`xc`) contract

A block declaring `xc_count > 0` owns a continuous-state vector the *solver itself*
numerically integrates — RK4, one independent integration per block, the same convention
`continuous_blocks::StateSpace::rk4_step` uses — as opposed to the opaque `void *state` blob,
which nothing but the block's own C code ever touches. Such a library exports **two more**
functions *instead of* `cscript_output`:

```c
void cscript_derivative(void *state, const double *in, int in_len,
                        const double *xc, int xc_len, double *xc_dot, int xc_dot_len);
void cscript_output_xc(void *state, const double *in, int in_len, double dt,
                       const double *xc, int xc_len, double *out, int out_len);
```

`cscript_derivative` is a pure vector-field evaluation (`dxc/dt = f(t, u, xc, xd)`) — must
not mutate `state`/`in`/`xc`, and may be called up to four times per accepted step (RK4's own
`k1..k4` stages). `cscript_output_xc` replaces `cscript_output`, with read access to this
step's already-integrated `xc`. `xc` itself lives in `dae-runtime`'s own `BlockState`, not
inside the opaque blob — "the one place this contract breaks the 'everything is opaque to
Rust' rule, and deliberately so": the solver's RK4 stepper needs ordinary vector arithmetic
on it between stages. Its initial value is always the zero vector, and cloning it is a plain
`Vec<f64>` clone needing no C-side involvement.

### The optional discrete-state update function

`cscript_output`/`cscript_output_xc` may mutate `state` themselves — fine for the common
case, but it conflates *computing this step's output* with *committing state forward*. The
optional `cscript_update` is the dedicated place for the latter:

```c
void cscript_update(void *state, const double *in, int in_len, double dt,
                    const double *xc, int xc_len);
```

Called once per resolved (or per-sample-period, under `ts=`) step, immediately after the
output call. Absent symbol ⇒ the block keeps updating `state` inside the output function,
exactly as before this function existed. The rationale was recorded when it was added:
`docs/journal/2026-08.md`, 2026-08-28 08:10.

### The optional block-controlled sample time

A block whose next execution time is only known at runtime (a modulator whose next switching
instant depends on a live computation) can export `cscript_next_sample_hit` alongside a
netlist `ts=variable` — the callback lets the block report its own requested next-hit time
each time it runs. Fixed-rate blocks use plain `ts=`/`freq=` instead; a library declared
`ts=variable` without the export fails up front with
`CScriptRequiresNextSampleHitForVariableSampleTime` (checked once, before any step runs).
Added 2026-08-28 08:51, together with the `to=` phase offset.

## `cscript_clone` and adaptive stepping

Adaptive step-size control (the default when `--dt` is omitted) retries a rejected trial step
from an independent copy of every block's state: the solver clones the whole `block_states`
vector before each trial, and discards a rejected trial's clone wholesale. An opaque C state
pointer cannot be deep-copied without the library's own help — so a `CScript` block whose
library doesn't export `cscript_clone` fails at the first adaptive step with
`CScriptRequiresCloneForAdaptiveStep { block_name: ... }`, a clean error rather than a
silently corrupted run. The two fixes, both documented in the Component Reference's CScript
entry: export `cscript_clone`, or run with a fixed `--dt`. The same clone-or-fixed-step
reasoning applies to the `xc` contract's "a rejected trial's mutation never survives"
guarantee.

## Source material this was adapted from

- `crates/cscript-ffi/src/lib.rs` — module doc (the full contract above), `CScriptRegistry`/
  `CScriptError`.
- `AGENTS.md` — "Numeric stack" (the Rust-throughout rule and its exception).
- `book/user-guide/src/cscript.md` — the user-facing summary, linked not duplicated.
- `docs/journal/2026-08.md` — 2026-08-27 12:00 (xc contract), 2026-08-28 08:10
  (`cscript_update`), 2026-08-28 08:51 (`cscript_next_sample_hit`/`to=`).
