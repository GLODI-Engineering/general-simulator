# The Python escape hatch

`kind=pyblock` and `kind=pyfunc` embed a Python interpreter in-process (via PyO3) and call into
a user-supplied `.py` file once per step — the Python-hosted counterpart to [the CScript escape
hatch](cscript.md), for reaching for `numpy`/`scipy`/an existing Python model instead of
compiling a `.so`. As with `cscript`, check the [Component Reference](component-reference.md)
first — composing existing blocks is almost always simpler than writing and debugging Python
against this contract.

Two block kinds, one chapter, because they're the same escape hatch at two different altitudes:

- **`kind=pyfunc`** — a plain, stateless function, called with each input as its own positional
  argument. No `state`, no `t`/`dt`, nothing to remember between calls. The right choice for a
  lookup table, a truth table, a static mapping from inputs to outputs.
- **`kind=pyblock`** — a persistent per-instance Python object carried across steps, the same
  `start`/`output`/`update`/`xc_count` shape `cscript` has. The right choice for anything with
  real memory, or that needs the solver to integrate a continuous state on its behalf.

## This requires a `--features python` build

**Unlike `cscript`, this is not on by default.** `pyblock`/`pyfunc` need a discoverable
Python/`libpython` at *build* time (PyO3's `auto-initialize` feature), so `crates/dae-runtime`
gates them behind an optional Cargo feature, and `crates/general-simulator-cli` forwards it:

```bash
cargo build --release -p general-simulator-cli --features python
```

Built without that flag, any `kind=pyblock`/`kind=pyfunc` netlist fails at the first step with:

```text
PythonSupportNotCompiledIn { block_name: "<name>" }
```

(the CLI's raw `Debug` formatting of `dae_runtime`'s own error type — searchable verbatim). If
you only need the C escape hatch, `kind=cscript` already covers "dynamically-loaded custom
block" without this dependency at all.

No `python -m venv`/`requirements.txt` story is built in: the interpreter PyO3 embeds is
whatever Python your build found (its own standard library included), and `import numpy`/
`import scipy` inside a `.py` file's `start()` works exactly like it would from a normal Python
script, using whatever's importable from that same interpreter's environment — there's no
separate dependency-isolation mechanism layered on top by this project.

## `kind=pyfunc`: stateless functions

### The contract

A `.py` file defining one plain function, matched by position, never by parameter name:

```python
def add_one(x):
    return x + 1.0
```

Called as `f(*inputs)` — each declared `inputs=` entry (or the single `in=` signal) is its own
positional argument, never bundled into one list. Works transparently whether the function
declares named parameters, `*args`, or a mix. Return a single value, or a tuple/list matching
`outputs=`'s declared count.

### A complete minimal example

`doc-verify/pyfunc/add.py`:

```python
def add_one(x):
    return x + 1.0
```

Wired in from `doc-verify/pyfunc/example.cir`:

```text
SRC kind=const value=3
G1 kind=pyfunc path=doc-verify/pyfunc/add.py function=add_one in=SRC
```

Verified end to end against a `--features python` build (`cargo build --release
-p general-simulator-cli --features python`, then `general-simulator doc-verify/pyfunc/example.cir
--mode transient --tfinal 1e-4 --dt 1e-5`) — real CLI stdout, `SRC=3` giving `G1=4` at every
step:

```text
t,SRC,G1
0.00001,3,4
0.00002,3,4
0.000030000000000000004,3,4
...
0.0001,3,4
```

`inputs=A,B` calling `f(A, B)` as two genuinely distinct positional arguments (not a swapped or
bundled pair) is exercised separately by `doc-verify/pyfunc/example_two_inputs.cir`.

### Parameters

- `path=<path>` — the `.py` file containing `function`. Whitespace-tokenized like every other
  field — `"double-quote"` a path containing whitespace (`path="my libs/add.py"`).
- `function=<name>` — required, no default; the function called as `f(*inputs)`.
- `outputs=<name1,name2,...>` — this block's own output names; defaults to a single output
  aliasing the block's own `.name`.
- `in=<signal>` or `inputs=<sig1,sig2,...>` — exactly one of these.
- `ts=<f64>` / `freq=<f64>` [`to=<f64>`] — optional fixed sample period, mutually exclusive,
  same zero-order-hold convention as `cscript`'s own `ts=`/`freq=` (see [the CScript
  chapter](cscript.md#outputs-and-tsfreq)). `ts=variable` is **rejected at parse time** here —
  a stateless function call has no instance to remember a requested next-hit time against:

  ```text
  ts=variable is not available for kind=pyfunc (a stateless function call has no instance to
  remember a requested next-hit time against) -- use kind=pyblock instead
  ```

There is no `xc_count=` for `kind=pyfunc` at all — a purely stateless function has nothing for a
continuous state to mean; reach for `kind=pyblock` if you need one.

## `kind=pyblock`: stateful blocks

### The Python-side contract, summarized

Full detail lives in `crates/pyblock-ffi/src/`'s own module doc comment and
`book/dev-guide/src/python-blocks.md` — this section is the "what do I need to write" summary.

**Required, always:**

```python
def start():
    """Called once, when this block instance is created. Return any Python object (a dict, a
    plain class instance, None, ...) as this instance's own persistent state. Put every import,
    every precomputed table, every one-time-expensive thing here -- never in output()."""
    ...
```

**The plain, single-function contract** (the common case — `xc_count=0`, the default):

```python
def output(state, t, dt, inputs):
    """Called once per resolved step (or once per ts=/freq= sample hit). `inputs` is a list, one
    entry per declared inputs= signal, each a float or a numpy array. Return a float, or a
    tuple/list matching outputs='s declared count."""
    ...
```

**Optional:**

- `def update(state, t, dt, inputs): ...` — called immediately after `output()` on every step
  the block actually runs, a dedicated place to advance discrete state separately from
  computing this step's output. If absent, `output()` is expected to mutate `state` itself,
  exactly as before this existed.
- `def next_sample_hit(state, t, dt, inputs): ...` — **required** when this block declares
  `ts=variable` (never called otherwise); returns the number of seconds, relative to this call,
  until the block should next run. A missing `next_sample_hit` with `ts=variable` declared is a
  load-time error.

### A complete minimal example

`doc-verify/pyblock/gain.py` — a fixed ×2 gain, no internal state needed at all:

```python
def start():
    return None

def output(state, t, dt, inputs):
    return inputs[0] * 2.0
```

Wired in from `doc-verify/pyblock/example.cir`:

```text
SRC kind=const value=3
G1 kind=pyblock path=doc-verify/pyblock/gain.py in=SRC
```

Verified end to end against a `--features python` build — real CLI stdout, `SRC=3` giving
`G1=6` at every step:

```text
t,SRC,G1
0.00001,3,6
0.00002,3,6
0.000030000000000000004,3,6
...
0.0001,3,6
```

Start from this fixture (or `doc-verify/pyblock/gain.py`/`example.cir` directly) rather than
writing a `.py` file from scratch.

### `outputs=` and `ts=`/`freq=`

Same shape as `cscript`'s own — see [that chapter's section](cscript.md#outputs-and-tsfreq) for
the full explanation of the zero-order-hold convention and `ts=variable`. `outputs=` defaults to
a single output aliasing the block's own name; `ts=`/`freq=`/`to=` fix a sample period
independent of the resolved circuit step; `ts=variable` hands scheduling to `next_sample_hit`
above.

### The optional continuous-state (`xc`) contract

If your block's state is more naturally expressed as ODE state the *solver* should integrate
(rather than something you hand-integrate yourself inside `output()`), set `xc_count=<usize>`
and export a different pair of functions *instead of* `output`:

```python
def derivative(state, t, inputs, xc):
    """xc: numpy array. Return dxc/dt, array-like, the same length as xc. Called up to four
    times per step (RK4's own k1..k4) -- must not mutate state/inputs/xc."""
    ...

def output_xc(state, t, dt, inputs, xc):
    """Produces this step's actual output from the now-integrated xc."""
    ...
```

`derivative()`'s return value always gets the "array-like" treatment, never the bare-scalar
shortcut `output()`/`output_xc()` get for a single return value — a length-1 continuous state
is still a state *vector* conceptually, even when the block itself only has one output.

Verified end to end against its own analytic solution: `doc-verify/pyblock/xc_decay.py`
implements $\dot{x}_c = 1 - x_c$, starting at rest ($x_c(0) = 0$), so $x_c(t) = 1 - e^{-t}$:

```python
def start():
    return None

def derivative(state, t, inputs, xc):
    return [1.0 - xc[0]]

def output_xc(state, t, dt, inputs, xc):
    return xc[0]
```

wired from `doc-verify/pyblock/xc_example.cir` with `xc_count=1`. Run against
`--tfinal 0.01 --dt 0.001`, the block's own output at $t=0.01$ matches
$1 - e^{-0.01} \approx 0.0099501663$ to within $10^{-6}$ — confirmed by `test_pyblock.py`'s own
`test_xc_count_continuous_state_block_integrates_correctly`.

There's also an optional `update(state, t, dt, inputs, xc)` under the `xc` contract — the same
`update` name as the plain contract, just with `xc` (this step's already-integrated continuous
state) as one extra positional argument, called immediately after `output_xc()`.

## Adaptive step-size control: no `clone` contract needed

**This is a real, deliberate difference from `cscript`.** `cscript` requires the C author to
hand-write `cscript_clone` (a deep copy of their own opaque struct) before a block can run under
adaptive stepping, and fails immediately without it. `kind=pyblock` needs no equivalent
opt-in: each instance's Python state is cloned with `copy.deepcopy`, a **generic** deep copy
that works on ordinary Python state (numbers, lists, dicts, numpy arrays, plain objects)
automatically, with no author-side export required. Confirmed directly against this build:
`doc-verify/pyblock/example.cir` (which exports no clone-related function at all) runs
correctly under adaptive stepping (no `--dt` flag) without error.

If your own state genuinely can't be deep-copied (an open file handle, a live socket),
`deepcopy` raises a normal Python exception, surfaced as a clear `DaeError::PyBlock` naming the
block and the Python traceback — not a panic, not silent corruption, and not the upfront
construction-time rejection `cscript`'s missing-clone case gets (since there's no way to know in
advance whether a given object will deep-copy cleanly).

`kind=pyfunc` clones even more trivially: since a `pyfunc` instance carries no per-call state at
all, cloning it is just cloning the function handle — unconditionally cheap and infallible,
never a `deepcopy` call.

## What isn't checked

A `.py` file missing a required function (`start`/`output`, or `derivative`/`output_xc` under
`xc_count>0`), raising an exception, or returning the wrong arity surfaces as a
Python-traceback-bearing failure from `pyblock_ffi` at *call* time, not at netlist-parse time —
there's no way to validate a Python file's actual callable surface before trying to call it. Get
the minimal example above working first, then extend it, rather than debugging a hand-written
`.py` file and the netlist wiring simultaneously.
