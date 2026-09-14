# Block library overview

Every `kind=` value the parser accepts is one entry in the table below: the electrical PWL
devices, the signal-domain blocks, the two physical/signal converters that cross between them,
and — a special case handled at the CLI level, never by the solver — `kind=measure`. The
authoritative list is the `match kind { ... }` dispatch in
`general-mna/src/system_builder.rs`'s `build_kind` (plus `crates/general-simulator-cli/src/measure.rs`,
which recognizes and strips `kind=measure` lines before `general-mna` ever sees them — see
`measurements.md`); the one-line descriptions and field signatures are the Component Reference
entries generated from `general-mna/src/block_graph.rs`'s doc comments. Where the two ever
disagree, the parser is right.

The organizing idea, stated once so it doesn't surprise you later: **each block does one job
and is wired to the others by the caller.** An error signal is a `sum` block's own output, a
frequency-modulated PWM carrier is `sum -> pid -> sum -> pspwm` — there is no monolithic
"PID controller with built-in PWM" block, and there never will be: `general-mna/src/block_graph.rs`'s
module doc comment is explicit that a fused block "bakes a specific topology together," which
is the one thing the block graph exists to avoid. The same idea is why the waveform-arithmetic
function library (`cos`, `sin`, `atan2`, `if`, `limit`, ...) is one row of this table rather
than one row per function — see `sources-and-math.md`.

"Inputs" counts block-graph inputs (named by `in=`/`inputs=`/`in1=`...). An electrical device's
"inputs" are its netlist nodes, not block signals; a `sig2phys` converter's output is consumed
by *name* (as a `V`/`I` source's literal or a gate's `ctrl=`), never wired into a node.

| `kind=` | What it is | Inputs | Outputs | Detail chapter |
|---|---|---|---|---|
| `ideal_diode` | 3-segment PWL diode (breakdown / leakage / forward), the default when `kind=` is omitted | — (device: 2 nodes) | — (device) | `pwl-devices.md` |
| `ideal_switch` | gated-on channel (`r_on`) falling back to its body diode when gated off | — (device: 2 nodes, `gate=block ctrl=<name>`) | — (device) | `pwl-devices.md`, `gate-bindings.md` |
| `const` | fixed scalar or vector value | 0 | 1 | `sources-and-math.md` |
| `time` | the current step's simulated time (seconds) | 0 | 1 | `sources-and-math.md` |
| `pwc` | piecewise-**constant** source (step schedules), optional `repeat=true` | 0 | 1 | `sources-and-math.md` |
| `pwl` | piecewise-**linear** source (real SPICE PWL semantics), optional `repeat=true` | 0 | 1 | `sources-and-math.md` |
| `sinwave` / `pulsewave` / `expwave` / `sffmwave` | the electrical domain's `SIN`/`PULSE`/`EXP`/`SFFM` source forms as signal references | 0 | 1 | `sources-and-math.md` |
| `sum` | weighted sum, one `signs=` entry per `inputs=` entry | N (one per term) | 1 | `sources-and-math.md` |
| `gain` | scale factor, or `M x N` matrix-vector product | 1 | 1 | `sources-and-math.md` |
| `product` | multiplies all inputs | N | 1 | `sources-and-math.md` |
| `saturation` | clamps to `[-limit, limit]` | 1 | 1 | `sources-and-math.md` |
| `table` | linear interpolation through a fixed `(x, y)` table | 1 | 1 | `sources-and-math.md` |
| `abs`, `cos`, `sin`, `tan`, `exp`, `ln`, `log10`, `sqrt`, `sinh`, `cosh`, `tanh`, `asin`, `acos`, `atan`, `asinh`, `acosh`, `atanh`, `floor`, `ceil`, `round`, `int`, `sgn`, `u`, `uramp`, `buf`, `inv` | unary waveform-arithmetic functions (`in=`) | 1 | 1 | `sources-and-math.md` |
| `atan2`, `anglewrap`, `hypot`, `pow`, `pwr`, `pwrs`, `min`, `max` | binary waveform-arithmetic functions (`in1=`, `in2=`) | 2 | 1 | `sources-and-math.md` |
| `if`, `limit` | ternary: threshold select; clamp to the span of two bounds (`in1=`, `in2=`, `in3=`) | 3 | 1 | `sources-and-math.md` |
| `pid` | PID with filtered derivative and anti-windup (fixed or dynamic clamp) | 1 (`Fixed` clamp) or 3 (`Dynamic`: `error, clamp_lo, clamp_hi`) | 1 | `dynamic-blocks.md` |
| `statespace` | arbitrary `(A, B, C, D)` block, genuinely MIMO | flattened concatenation of its declared inputs | 1 per `C` row | `dynamic-blocks.md` |
| `tf` | rational `N(s)/D(s)`, compiled to state space | 1 | 1 | `dynamic-blocks.md` |
| `discretestatespace` / `discretetf` / `discretepid` | the discrete-domain counterparts of the three above (`ts=`/`freq=` required) | same as their continuous counterparts | same | `dynamic-blocks.md` |
| `vco` | bare `[0, 1)` oscillator ramp from a frequency command | 1 | 1 | `dynamic-blocks.md` |
| `pwm` | PWM Modulator 1: fixed frequency, block-driven duty, active-high complementary pair with dead time | 1 (`duty`) | 2 (`main`, `complement`) | `gate-bindings.md` |
| `pspwm` | PWM Modulator 2: frequency+phase+duty block-driven, own phase-integration state | 3 (`freq`, `phase`, `duty`) | 2 (`main`, `complement`) | `gate-bindings.md` |
| `hysteresis` | Schmitt-trigger comparator (bang-bang current-mode control) | 1 | 1 | `dynamic-blocks.md` |
| `and`, `or`, `xor`, `nand`, `nor`, `xnor` | N-input combinational logic gate (`inputs=`) | >= 2 | 1 | Component Reference |
| `not` | 1-input logic inversion (`in=`) | 1 | 1 | Component Reference |
| `srlatch` | level-triggered set/reset latch | 2 (`set`, `reset`) | 1 | Component Reference |
| `dff` / `tff` / `jkff` | edge-triggered D/T/JK flip-flop (`clk=` plus `d=`/`t=`/`j=`+`k=`, optional synchronous `reset=`) | 2 (D/T), 3 (JK), +1 with `reset=` | 1 | Component Reference |
| `counter` | clocked up/down counter (optional `up_down=`, `modulus=`, `reset=`) | 1 (`clk`), +1 each optional control input | 1 | Component Reference |
| `cscript` | user-supplied precompiled C shared library, one call per step | 1 (`in=`) or N (`inputs=`) | 1..N (`outputs=`) | `cscript.md` |
| `pyblock` | stateful Python function (`start`/`output`/optional `update`), Python-hosted `cscript` analog | 1 or N | 1..N | dev guide `python-blocks.md` |
| `pyfunc` | stateless pure Python function, one call per step | N (each a positional argument) | 1 | dev guide `pyfunc-blocks.md` |
| `octfunc` / `octblock` | the Octave-hosted stateless/stateful analogs (`octave-cli` subprocess, no linking) | 1..N | 1..N | dev guide `octave-blocks.md` |
| `clarke` / `clarkeinv` | abc <-> alpha/beta/zero transforms | 3 | 3 | `coordinate-transforms.md` |
| `park` / `parkinv` / `clarkepark` / `clarkeparkinv` | rotating-frame transforms | 4 | 3 | `coordinate-transforms.md` |
| `pmsm` | permanent-magnet synchronous motor in the rotor d/q frame, own RK4 stepper | 3 (`vd`, `vq`, `t_load`) | 4 (`id`, `iq`, `omega_m`, `theta_e`) | `pmsm.md` |
| `phys2sig` | the only way a circuit quantity (`V(node)`/`I(branch)`) enters the signal domain | 0 | 1 | `signals.md` |
| `sig2phys` | the only legal target for a `V`/`I` source's magnitude or a gate's `ctrl=`; identity pass-through, consumed by name | 1 | 1 | `signals.md`, `gate-bindings.md` |
| `measure` | ngspice `.measure`-style post-processing over a completed trace — stripped by the CLI before the solver sees the file; never a block | — | — (writes `<netlist stem>.log`) | `measurements.md` |

Two rows are worth reading before you write your first netlist: `ideal_switch` (its
`(drain, source)` node-order convention — `pwl-devices.md` says "read this before wiring a
switch") and `sig2phys` (nothing else may appear in a gate's `ctrl=` or a source's literal
field; `gate-bindings.md`'s "Two real errors worth knowing before you hit them" section is the
short version).

## Source material this was adapted from

- `general-mna/src/system_builder.rs` — `build_kind`'s match arms, the authoritative
  `kind=` list (every row of the table was checked against a match arm, not assumed).
- `general-mna/src/block_graph.rs` — the `BlockInstance` doc comment's input-count
  conventions and the module doc's "each block does one job" organizing idea.
- `book/user-guide/src/component-reference.md` — the generated one-line Purpose per component.
- `crates/general-simulator-cli/src/measure.rs` — `kind=measure` recognized and stripped at
  the CLI level, never seen by `general-mna`.
- The `book/user-guide/src/*.md` detail chapters named in the table's last column.
