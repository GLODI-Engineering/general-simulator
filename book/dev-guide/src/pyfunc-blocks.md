# Python pure-function blocks: design

*(Implemented this session, as a genuinely separate, additive block kind alongside `pyblock` —
not a mode of it. Nothing in `pyblock-ffi`'s existing `PyBlockRegistry`/`PyBlockInstance`
contract, docs, or tests was touched to build this.)*

## What this is, and what it isn't

`kind=pyblock` (see `python-blocks.md`) mirrors `cscript`'s own `start`/`output` lifecycle: a
persistent per-instance `state` object, `t`/`dt` passed to every call, optional `derivative`/
`output_xc` for an RK4-integrated continuous state. That's the right shape for a block that
genuinely has state or needs solver-integrated dynamics.

Plenty of real controller logic has none of that — a lookup table, a truth table, a static
mapping from inputs to outputs, no memory between calls. The user's own motivating example is
exactly this shape: a gate/action-qualifier pattern generator keyed only on the current phase
angle, structurally identical to a plain function block in other block-diagram tools — named
inputs, named outputs, a body that's just ordinary imperative code with no per-call state:

```
function [AQCTLA,AQCTLB] = compute_action_qualifier_180_degree(phase_degree)
    phase_shift = mod(phase_degree,360);
    if phase_shift == 0
        AQCTLA = 9; AQCTLB = 6;
    elseif phase_shift > 0 && phase_shift < 180
        AQCTLA = 2066; AQCTLB = 1057;
    ...
```

`kind=pyfunc` gives that shape a direct Python equivalent: a plain function, named parameters
(or `*args`), a single return value or a tuple. No `start`, no `state`, no `t`/`dt`, no
`xc_count` — there's nothing for any of those to mean for a function with no memory.

## The contract

```python
def compute_action_qualifier_180_degree(phase_degree):
    """Called once per resolved step (or once per sample period under ts=), with each declared
    inputs= entry as its own POSITIONAL argument -- f(*inputs), never bundled into one list the
    way pyblock's output(state, t, dt, inputs) is. Works transparently whether the function
    declares named parameters, *args, or a mix -- inputs= is matched by position only, never by
    parameter name. Return a single value, or a tuple/list matching outputs='s declared count."""
    phase_shift = phase_degree % 360
    ...
    return AQCTLA, AQCTLB
```

Netlist usage (the user's own example, ported):

```
AQ kind=pyfunc path=gate_pattern.py function=compute_action_qualifier_180_degree \
   in=PHASE outputs=AQCTLA,AQCTLB
```

`function=` is **required** here (unlike `pyblock`, which always calls a fixed `output` name) —
the whole point of this block is calling a *descriptively named* function, so there's no
sensible default to fall back to.

A `Vector` input arrives as its own `numpy.ndarray` positional argument, never flattened
together with neighboring scalars — `f(*inputs)`, one Python object per declared `inputs=`
entry, exactly mirroring how `pyblock` keeps each input its own list entry rather than
flattening to one big array.

## Why a new block kind, not a mode of `pyblock`

The first pass at this implemented it as an auto-detected branch inside the existing `PyBlock`
contract. Rejected: the two calling conventions are different enough (`f(state, t, dt, inputs)`
vs. `f(*inputs)`) that any auto-detection needs to inspect the loaded function's own signature
at load time, adding a real branch of logic — and every future reader of `pyblock`'s own module
doc comment, error variants, and tests would have to hold both conventions in mind to know which
one an edit affects. Keeping them structurally separate (`BlockKind::PyFunction`, a private
`pyblock_ffi::pure_function` submodule, `PyFunctionRegistry`/`PyFunctionInstance`, `kind=pyfunc`,
their own test files) means the existing `pyblock` contract is provably unaffected by this work —
confirmed by an empty `git diff` on every one of its own files.

The new code still **reuses** `pyblock`'s private helpers by calling them, not duplicating them:
`PyBlockError`, `PyInput`, `exec_fresh_module` (same compiled-source-cache-per-path, fresh-
namespace-per-instance pattern `pyblock` already has), `extract_outputs`, `py_traceback`. Only
the argument-building (`f(*inputs)` as a `PyTuple`, vs. `pyblock`'s own `f([inputs])` as a
`PyList`) is genuinely new.

## A further advantage over both `cscript` and `pyblock`: cloning is always trivial

`PyFunctionInstance::clone` is `Py::clone_ref` on the function handle alone — never
`copy.deepcopy`, never fallible, because there is no per-instance state to copy at all. Where
`pyblock`'s own `try_clone` calls `copy.deepcopy` on a namespace (fast, but a real Python call
that could in principle raise, per `python-blocks.md`'s own "State and cloning" section), a
`pyfunc` instance clone is unconditionally cheap and infallible — the strongest position of any
block kind this simulator has for adaptive-step cloning.

## What's deliberately not built

- No `state`/`t`/`dt`/`xc_count` at all — genuinely out of scope for a stateless function, not
  an oversight. A netlist author who needs persistent state or solver-integrated dynamics should
  reach for `kind=pyblock` instead.
- No parameter-name matching against netlist signal names — `inputs=` binds by position only,
  matching `*args`/`**kwargs`-style Python calling conventions where the *names* a function
  chooses are its own business. This is also why `**kwargs` specifically isn't reachable from a
  netlist: there's no netlist syntax for keyword arguments, only an ordered `inputs=` list.
