# Python function blocks: design

*(Implemented this session — this chapter records the design and the measurements that
grounded it, mirroring `vector-signals.md`'s own before/after structure.)*

## What this is, and what it isn't

A `kind=pyblock` block lets a netlist author write an arbitrary Python function, called once
per resolved step (or once per sample period under `ts=`, the same zero-order-hold convention
`cscript` already has) — the same role a scripting-language code block plays in any block-
diagram simulation tool: an escape hatch to a full language when the existing block library
doesn't cover something, without leaving the block-diagram model.

**This is a different integration from `general-mna`'s existing `python` feature.**
`general-mna/src/python.rs` compiles `general-mna` itself *into* a Python extension module
(`#[pyfunction]`/`#[pymodule]`, PyO3's now-deprecated `extension-module` feature) so a Python
program can call *into* this Rust engine. `pyblock` is the opposite direction: the Rust
simulator embeds its own Python interpreter (PyO3's `auto-initialize` feature) and calls *into*
user-supplied Python code. The two features are mutually incompatible in one crate (one assumes
Python hosts you, the other assumes you host Python) — `pyblock` lives in a new sibling crate,
`crates/pyblock-ffi`, the direct structural analog of `cscript-ffi`.

## The contract — deliberately mirrors `cscript_ffi`

```python
def start():
    """Called once, when this block instance is created. Return any Python object (a dict, a
    plain class instance, ...) as this instance's own persistent state, or None. Put every
    import, every precomputed table, every one-time-expensive thing here -- never in output()."""
    ...

def output(state, t, dt, inputs):
    """Called once per resolved step (or once per sample period under ts=). `inputs` is a list,
    one entry per declared `inputs=` signal, each a float (Scalar) or a numpy array (Vector).
    Return a float, or a tuple/list of floats matching `outputs=`'s own declared count."""
    ...

# Optional, for a continuous state the solver itself RK4-integrates instead of state.py
# hand-managing it (xc_count > 0) -- mirrors cscript_derivative/cscript_output_xc exactly:
def derivative(state, t, inputs, xc):
    """xc: numpy array. Return dxc/dt, array-like, same length."""
    ...

def output_xc(state, t, dt, inputs, xc):
    ...

# Optional, either contract -- see "The optional discrete-state update function" below:
def update(state, t, dt, inputs):        # plain contract
    ...
def update(state, t, dt, inputs, xc):     # xc contract -- one extra positional argument
    ...
```

Same split as `cscript`, same reasons: `start()` is where import/compile/precompute costs get
paid exactly once; `output()`/`derivative()`/`output_xc()` run every step and should do as
little as possible beyond the actual per-step computation.

## The optional discrete-state update function

`output()`/`output_xc()` are allowed to mutate `state` themselves — fine for the common case,
but it conflates two different things: computing this step's output from the block's current
state, and committing that state forward to the next step. Some block-diagram tools' own
code-block feature keeps these deliberately separate (an output function expected to be
side-effect-free, plus a dedicated update function that's the only place discrete state actually
advances); `pyblock` now offers the same split as an **entirely optional** function, mirroring
`cscript`'s own `cscript_update`. If a `.py` file defines `update`, it's called once per resolved
(or per-sample-period, under `ts=`) step, immediately after `output()`/`output_xc()` for that
same call. There's no separate `update_xc` name — the same `update` function just receives one
more positional argument (`xc`, this step's own already-integrated continuous state) under the
`xc` contract, mirroring `output`/`output_xc`'s own split being about the return value rather
than the input shape here. If `update` is absent, a block is expected to keep updating `state`
directly inside `output()`/`output_xc()`, exactly as before this function existed — nothing about
the existing contract changes if you never add one.

## Sample time: fixed period, phase offset, or block-controlled

`ts=`/`freq=` fix a sample period at netlist-parse time (`sample_time=None`, the default, runs
every resolved circuit step instead). Two refinements: `to=X` delays the *first* hit to `t=X`
instead of the period's own implicit anchor (`0 <= X < period`; omitted, the default, preserves
the pre-existing behavior — the first hit lands on the very first resolved step regardless of
`period`'s own value, not one full period in). `ts=variable` hands scheduling to the block
itself — for a next-event time known only at runtime, not at parse time — and requires the `.py`
file to define one more function:

```python
def next_sample_hit(state, t, dt, inputs):        # plain contract
    """Required when this block declares ts=variable (never called otherwise). Called
    immediately after output(). Returns the number of seconds -- a duration, relative to this
    call, not an absolute time stamp -- until this block should next be executed. Must be > 0."""
    ...
def next_sample_hit(state, t, dt, inputs, xc):    # xc contract -- one extra argument
    ...
```

A missing `next_sample_hit` when `ts=variable` is declared is a load-time error, not a silent
fallback — there would otherwise be no way to know when to run the block at all.

A block that adopts this split gets the same guarantee every other synchronous/Moore-style block
in this project's own block graph already has: this step's `output()` sees only *last step's*
committed state, never a value `update()` computed later in the same call — useful when the
distinction between "what this step reports" and "what state means going forward" actually
matters (e.g. porting an existing controller that was already written with separate output/update
functions in some other tool, where collapsing them into one function would be a real behavioral
change, not just a stylistic one).

**Deliberately scalar-only outputs**, matching `cscript`'s own design decision (per
`vector-signals.md`): `outputs=NAME1,NAME2,...` declares N separately-named scalar values, not
one Vector-valued block name. Kept consistent with `cscript`/`CoordinateTransform`/`Pmsm` rather
than reopening that question for this one block kind.

**Inputs flatten the same way**: `inputs=` may mix scalar and vector signals, concatenated in
order — reusing `dae-runtime`'s existing `flatten` helper, no new mechanism.

## Measured, not assumed: real numbers from this machine

Guessed at "microseconds per call" before measuring; the real numbers (Python 3.14.4, PyO3
0.29.2, `cargo run --release`, 200,000 iterations, warmed up) are better than that guess:

| Operation | Measured |
|---|---|
| Bare `Python::attach` (GIL acquire/release, no call) | ~95 ns |
| Scalar function call (tuple args, `f64` extract) | ~150–300 ns |
| numpy-array round trip (`PyArray1` in, array out) | ~0.9–1.4 µs |

Order-of-magnitude conclusions from these:
- **Acquiring the GIL per call is cheap enough not to design around.** No need to hold the GIL
  across the whole transient loop (which would also mean holding a global lock for the run's
  entire duration) — a plain `Python::attach` at each individual `pyblock` call site is simpler
  and the measured cost is negligible next to anything the user's own code will do.
- **A scalar `pyblock` call costs roughly the same order of magnitude as `cscript`'s own C FFI
  call** (tens of nanoseconds) times a small constant factor — not the 100–1000x penalty a more
  pessimistic estimate (mine, before measuring) would suggest. For a run with 10⁴–10⁶ steps and
  one scalar `pyblock` in the loop, pure call overhead is milliseconds at most.
- **numpy round-trips cost several times more than a scalar call** (array wrapping/marshaling),
  still sub-microsecond — real, but small next to whatever vectorized numeric work the array
  round trip is *for*. The `sample_time`/`ts=` zero-order-hold option matters more here than raw
  per-call cost: a `pyblock` modeling a genuinely low-rate digital controller should run at its
  own real sample rate regardless, both for physical correctness and to skip work that doesn't
  need to happen every fine circuit step.
- **The dominant, controllable cost is the mistake this design specifically guards against**:
  importing numpy/scipy or compiling source *inside* `output()` instead of once in `start()`.
  The first `import numpy`/`import scipy...` in a process is genuinely slow (tens to a few
  hundred milliseconds — mostly shared-library loading and BLAS/LAPACK init), but CPython caches
  every import in `sys.modules`, so it's a one-time-per-*process* cost regardless of how many
  `pyblock` instances a netlist declares, *as long as* the import happens at `start()` time and
  not inside the per-step call.

## State and cloning: a real advantage over `cscript`

`cscript` requires the C author to hand-write `cscript_clone` (a deep copy of their own opaque
struct) for a block to be usable under adaptive stepping, and panics (documented, "checked once
up front") if it's missing. Python doesn't need this: `copy.deepcopy` is a **generic** deep copy
that works on ordinary Python state (numbers, lists, dicts, numpy arrays, plain objects)
automatically. `PyBlockInstance::try_clone` calls `copy.deepcopy` on the instance's own
namespace — no author opt-in required, and no `DaeError::PyBlockRequiresCloneForAdaptiveStep`-
style upfront rejection exists for this block kind at all. If a user's own state genuinely can't
be deep-copied (an open file handle, a live socket), `deepcopy` raises a normal Python
exception, surfaced as a clear `PyBlockError`, not a panic and not silent corruption.

**Per-instance isolation**: each `pyblock` instance gets its own Python namespace (a fresh
`globals()` dict), so two instances of the same `.py` file don't share module-level state or
`persistent`-style variables — while still sharing the *process-wide* `sys.modules` cache, so
numpy/scipy itself is genuinely initialized once regardless of how many `pyblock` instances
import it. The registry caches the *compiled code object* per file path (avoiding re-parsing the
same source for every instance) but `exec`s it into a fresh namespace per instance — "shared
code, independent state," the same relationship `cscript-ffi`'s registry has between one loaded
`.so` and N independent `cscript_start()` calls.

## Error handling: the other real advantage over `cscript`

A Python exception inside user code is a catchable `PyErr`, translated into a clear
`DaeError::PyBlock(PyBlockError::Exception(String))` naming the block and the Python traceback —
not undefined behavior, not a segfault. `cscript`'s raw C has none of this; a `pyblock` bug fails
loudly and diagnosably instead.

## Two real findings from building it

**`derivative()`'s return value is never given the bare-scalar shortcut `output()`/`output_xc()`
get.** First implementation reused one extraction helper for both, and a one-state `xc_count=1`
test failed with `TypeError: must be real number, not list` — `derivative()`'s own docstring
already promises "array-like, same length as `xc`" (unconditionally), but the shared helper
collapsed a length-1 return to "must be a bare `float`" the same way `output()`'s single-output
convention does. Fixed by giving `derivative()` its own extraction path (`extract_vector`,
always iterates) instead of `output()`'s own (`extract_outputs`, bare `float` shortcut only
below two outputs) — a length-1 continuous state is still a state *vector* conceptually, even
though a single scalar output is genuinely just a scalar.

**A `path=`/`lib=` field can't contain whitespace anywhere in the value**, including the
containing filesystem path — the netlist grammar's own field parser splits on whitespace with
no quoting support. Not new to this work (equally true of `cscript`'s own `lib=`), but this
session's own test fixtures happened to live under a path containing a space
(`.../Github Works/...`), surfacing it directly: `general-simulator-cli`'s own `pyblock`/
`cscript` tests now copy their fixture files into `std::env::temp_dir()` before referencing them
from netlist text, rather than referencing the crate's own source tree path directly.

## What's deliberately not built

- No code-generation/compilation story (a scripting-language code block in some other block-
  diagram tools can be compiled away to C for an embedded target; a `pyblock` cannot — it's
  always interpreted). Not a goal here: this simulator's own purpose doesn't need embedded-
  target deployment.
- No vector-valued `pyblock` *output* (see "deliberately scalar-only outputs" above) — consistent
  with, not a new exception to, the existing `vector-signals.md` decision.
- No thread pool / GIL-release-for-concurrency story — this simulator is single-threaded by
  design (one step at a time), so there's no contention to design around.
