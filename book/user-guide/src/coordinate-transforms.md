# Coordinate transforms (Clarke/Park) and PLL

The six Clarke/Park transforms and the `anglewrap` PLL-angle utility turn a three-phase or
rotating-frame control problem into something an ordinary [`kind=pid`](dynamic-blocks.md) can
regulate directly, instead of chasing a sine wave. The underlying math lives in
`crates/continuous-blocks/src/coordinate_transforms.rs`; the full field reference for each
`kind=` lives in the [Component Reference](component-reference.md#coordinate-transform-clarkepark).
This chapter is the conceptual tour: what each transform is for, and the standard PLL recipe.

## The six transforms

Every function here is a pure, stateless function of its instantaneous inputs — no integration,
no memory, unlike the [dynamic blocks](dynamic-blocks.md) chapter's PID/state-space/transfer
function:

| `kind=` | signature | conventional output names |
|---|---|---|
| `clarke` | `(a, b, c) -> (alpha, beta, zero)` | `alpha`, `beta`, `zero` |
| `clarkeinv` | `(alpha, beta, zero) -> (a, b, c)` | `a`, `b`, `c` |
| `park` | `(alpha, beta, zero, theta) -> (d, q, zero)` | `d`, `q`, `zero` |
| `parkinv` | `(d, q, zero, theta) -> (alpha, beta, zero)` | `alpha`, `beta`, `zero` |
| `clarkepark` | `(a, b, c, theta) -> (d, q, zero)` | `d`, `q`, `zero` |
| `clarkeparkinv` | `(d, q, zero, theta) -> (a, b, c)` | `a`, `b`, `c` |

`clarkepark`/`clarkeparkinv` are convenience compositions — `clarkepark(a,b,c,theta)` gives
exactly `park(clarke(a,b,c), theta)`, skipping the `alpha`/`beta` intermediate when you don't
need it as a separate signal.

Every transform is declared with `inputs=<sig1,sig2,...>` (3 entries for `clarke`/`clarkeinv`, 4
for the other four — they also take the angle `theta`, radians) and is **multi-output**: the
block's own name aliases the *primary* output, and `outputs=<name1,name2,name3>` optionally
names all three explicitly. Leaving `outputs=` unset auto-generates the rest from each
transform's own conventional names — a `kind=clarke` block named `PLL` with no `outputs=` gives
you `PLL` (`alpha`), `PLL_beta`, `PLL_zero`
(`crates/general-simulator-cli/src/main.rs`'s module doc comment).

**Amplitude convention**: the standard $\tfrac{2}{3}$ (non-power-invariant) scaling throughout —
a balanced three-phase signal of peak amplitude $A$ transforms to a `d`/`q` pair of magnitude
$A$, not $A\sqrt{3/2}$. If you're porting gains from a power-invariant-scaled reference (some
textbooks use the $\sqrt{2/3}$ form instead), rescale them; this project's own convention is
fixed and doesn't offer the other scaling as an option.

## `anglewrap`: the PLL angle-tracking half

`anglewrap(alpha, beta)` is $\operatorname{atan2}(\beta, \alpha)$ wrapped to $[0, 2\pi)$ — declared as an ordinary
two-argument waveform-arithmetic function (`kind=anglewrap in1=<alpha> in2=<beta>`, see
[Sources and math operations](sources-and-math.md)), not a `CoordinateTransform` variant,
because unlike the other six it's genuinely single-output; it's grouped with the coordinate
transforms conceptually (and implemented in the same source module) because it's the natural
angle-tracking counterpart that feeds a `kind=park`/`kind=clarkepark` block's own `theta` input.

**It is an instantaneous algebraic function, not a filtered or tracking PLL.** There's no
internal state, no settling transient, no loop filter of its own — `anglewrap`'s output at any
instant is exactly the angle of the `(alpha, beta)` vector at that instant, full stop. Don't
expect (or look for) a PLL-style lock-in transient from `anglewrap` alone; if your circuit's
`alpha`/`beta` is already clean, the tracked angle is exact from the very first sample. Any real
PLL *dynamics* — loop bandwidth, lock time, ripple rejection — come from whatever you build
around it (typically a `kind=pid` regulating the transform's own `q` output, as below), not from
`anglewrap` itself.

## A minimal SRF-PLL recipe

The standard synchronous-reference-frame PLL topology is `Clarke -> anglewrap -> Park`, with a
`kind=pid` closing the loop on the `q` output back into the tracked angle — a `Sum` block never
needed here, since Park itself already computes an angle *error* implicitly (its `q` output is
exactly zero when `theta` matches the input vector's true angle, and grows with the angle
error for small mistracking):

```text
* three-phase source feeding the PLL...
A kind=const value=1
B kind=const value=-0.5
C kind=const value=-0.5
CLARKE1 kind=clarke inputs=A,B,C
* anglewrap tracks the instantaneous angle from alpha/beta directly...
THETA_FF kind=anglewrap in1=CLARKE1 in2=CLARKE1_beta
* ...while a PID refines it via the Park q-error, closing the loop through prev: (see the
* Signals chapter for why a same-step reference back into the very Park block that produced
* q would be an algebraic loop):
PARK1 kind=park inputs=CLARKE1,CLARKE1_beta,CLARKE1_zero,prev:THETA_EST
PID1 kind=pid in=PARK1_q kp=... ki=... kd=0 n=1000 clamp_lo=-1e5 clamp_hi=1e5
THETA_EST kind=sum inputs=THETA_FF,PID1 signs=1,1
```

This sketch is deliberately schematic — the fully worked, numerically verified version (real
gains, a plot, and the actual three-phase test signal) lives in an internal three-phase PFC
experiment's write-up, Stage 1 Part
A; reproduce that experiment rather than re-deriving the gains from scratch.

## Cross-references

- [Sources and math operations](sources-and-math.md) — `anglewrap`'s own home in the
  waveform-arithmetic function table.
- [Dynamic blocks](dynamic-blocks.md) — the `kind=pid` closing the loop above.
- [Signals](signals.md) — why the loop above needs `prev:` at exactly the point it's used.
- [The PMSM block](pmsm.md) — the other place `clarkepark`/`park` show up constantly, feeding
  and fed by a motor's own `theta_e`.
