# Sources and math operations

The [Component Reference](component-reference.md) already has the full, authoritative field list
for every block in this chapter (generated directly from the real doc comments, kept current
automatically). This chapter is a guided tour through it — how the pieces fit together and which
one to reach for — not a restatement of each entry's own parameter table.

## Sources

- [`kind=const`](component-reference.md#constant) — a fixed value, scalar or vector, no state.
  The everyday setpoint/bias/offset block.
- [`kind=time`](component-reference.md#time) — the current simulated time as a signal, no fields,
  no inputs. This is the block that makes a genuinely time-varying signal buildable out of
  otherwise-stateless math blocks — nothing else lets a block see `t` directly.
- [`kind=pwc`](component-reference.md#piecewise-constant-source-pwc) — piecewise-**constant**:
  holds the last breakpoint's value until the next one. Step tests and staircase schedules.
- [`kind=pwl`](component-reference.md#piecewise-linear-source-pwl) — piecewise-**linear**:
  interpolates between breakpoints, real SPICE PWL semantics in the signal domain. Ramps and
  (with `repeat=true`) periodic triangle/sawtooth references — the electrical-domain `PWL`
  source has no repeat option, so a genuinely periodic piecewise-linear waveform is only
  available here.
- [`kind=sinwave`/`pulsewave`/`expwave`/`sffmwave`](component-reference.md#waveform-sources-sinwave--pulsewave--expwave--sffmwave)
  — the same field names, order, and defaults as SPICE's own `SIN()`/`PULSE()`/`EXP()`/`SFFM()`
  sources, reused directly so a `V`/`I` source and a signal-domain reference built from the same
  parameters produce bit-for-bit the same waveform.

`pwc` and `pwl` are deliberately named differently at the CLI level specifically to avoid the
ambiguity a shared `pwl` name would create between "holds between breakpoints" and
"interpolates between breakpoints" — worth remembering the distinction the two names encode.

### The $\sin(2\pi f t)$ recipe

No block sees `t` directly except `kind=time`, so building an arbitrary function of time out of
the stateless math blocks below always starts the same way — `time -> gain -> <fn>`. For a 1 kHz
sine ($2\pi \times 1000 = 6283.185307179586$):

```text
T kind=time
G kind=gain in=T k=6283.185307179586
S kind=sin in=G
```

This is the Component Reference's own verified [Time](component-reference.md#time) example, and
recurs constantly in worked examples throughout this book — worth internalizing once rather than
re-deriving each time you need a synthetic sinusoidal reference.

## Stateless math operations

- [`kind=gain`](component-reference.md#gain) — scales its input: a plain factor (`k=<f64>`,
  `Scalar->Scalar` or elementwise on a `Vector`), or a matrix (`k=[[...],...]`, the one math
  block that changes a signal's own length via a matrix-vector product).
- [`kind=sum`](component-reference.md#sum) — a weighted sum with one sign per input
  (`inputs=`/`signs=`) — the standard error junction (`signs=1,-1`), or (with fractional signs) a
  weighted average.
- [`kind=product`](component-reference.md#product) — multiplies all its inputs together.
- [`kind=saturation`](component-reference.md#saturation) — clamps a single input to a *symmetric*
  $[-\text{limit}, \text{limit}]$ band.
- [`kind=table`](component-reference.md#table-lookup) — linear interpolation through a fixed
  `(x, y)` breakpoint table, clamped (not extrapolated) past either end.

`sum` and `product` share one input-shape rule worth knowing once rather than per-block: all-
`Scalar` inputs give a `Scalar` output; otherwise every input must be a `Vector` of one common
length, reduced elementwise — a mix of `Scalar` and `Vector` inputs is rejected rather than
guessed at, since there's no unambiguous way to broadcast a lone scalar once more than one
vector is already present.

## The waveform-arithmetic function library

Beyond the named blocks above, any `kind=` naming a real-valued scalar function resolves to that
function as a block directly — there's no separate `kind=` list to maintain. Three arities, each
one Component Reference entry:

- [Unary](component-reference.md#waveform-arithmetic-functions-unary) (`in=`) — 26 functions:
  trig (`sin`/`cos`/`tan`/`asin`/`acos`/`atan`, hyperbolic variants), `exp`/`ln`/`log10`/`sqrt`,
  rounding (`floor`/`ceil`/`round`/`int`), `abs`/`sgn`, unit step/ramp (`u`/`uramp`), and a
  0.5-threshold buffer pair (`buf`/`inv`).
  - Two worth calling out because they're easy to guess wrong: `sgn` returns `0` at the origin
    (unlike `f64::signum`, which returns `1` there), and `uramp` is a ReLU ($x$ if $x>0$ else
    $0$), not a literal ramp generator.
- [Binary](component-reference.md#waveform-arithmetic-functions-binary) (`in1=`/`in2=`) — the
  `atan2`/`hypot`/`pow`-family, `min`/`max`, and `anglewrap` (the angle-tracking half of a
  synchronous-reference-frame PLL — see [Coordinate transforms](coordinate-transforms.md), which
  is where it's actually used even though it lives in this family since it's an ordinary
  two-argument function).
- [Ternary](component-reference.md#waveform-arithmetic-functions-ternary-if-limit) (`in1=`/`in2=`/`in3=`)
  — `if` (threshold select: `in2` while $\text{in1} > 0.5$, else `in3`) and `limit` (clamp `in1`
  to $[\min(\text{in2},\text{in3}), \max(\text{in2},\text{in3})]$ — unlike `kind=saturation`'s
  fixed $\pm\text{limit}$, both bounds here are themselves ordinary signals, so they can come
  from elsewhere in the graph).

Every member shares one input/error contract at its own arity, elementwise on `Vector` inputs
the same way `sum`/`gain` are.

## What's deliberately not here, and why

`continuous_blocks::waveform_arithmetic`'s own module doc comment argues this directly rather
than leaving it to be discovered by absence: no finite-difference derivative block, no
noise/random generator, no complex-data functions, and no Boolean/comparison operators — none of
those are stateless real-valued math the way everything above is. A derivative in particular is
a common enough request that it's worth stating plainly it's excluded on purpose rather than
simply missing; see the dev guide for the full rationale rather than re-arguing it here.

## Cross-references

[Dynamic blocks](dynamic-blocks.md) covers the *stateful* continuous-time blocks (`pid`,
`statespace`, `tf`) this chapter's sources and math operations typically feed into or read from.
