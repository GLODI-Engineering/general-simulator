# PWL devices: diodes and ideal switches

`general-mna` never stamps a real semiconductor's exponential I-V curve — no `D`-card model
physics, no BSIM MOSFET. Instead it stamps a piecewise-linear (PWL) companion model, solved
through the same LCP machinery as any other switching element. This chapter covers the two PWL
device kinds `general-simulator` ships: the ideal diode and the ideal switch built from it.
Field-by-field parameter tables for both already live in the
[Component Reference](component-reference.md); this chapter is the conceptual tour.

## `kind=ideal_diode`: the 3-segment curve

An ideal diode is a 3-segment piecewise-linear curve, continuous at both breakpoints by
construction (`crates/pwl-devices/src/ideal_diode.rs`):

```text
           g_breakdown          g_off (leakage)         g_on (forward)
  <---------------------|------------------------|-------------------->
                    v_breakdown                 v_th
```

- **`v <= v_breakdown`**: reverse-breakdown conduction, slope `g_breakdown`.
- **`v_breakdown < v < v_th`**: near-zero leakage, slope `g_off`.
- **`v >= v_th`**: forward conduction, slope `g_on`.

Both breakpoints are anchored so each segment meets its neighbor exactly, and `v_breakdown` must
be strictly below `v_th` (`IdealDiode::new` asserts this — see [Your first netlist](first-netlist.md)
for the example deck already using `g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.7 g_on=1`).
`v` is the diode's own $V = V_{\text{anode}} - V_{\text{cathode}}$, in netlist terms $V(n_1) -
V(n_2)$ for `D1 n1 n2 ...`.

### Picking parameters from a real datasheet

A real silicon diode's datasheet gives you roughly two numbers directly: a forward voltage drop
`V_F` at some rated current, and a reverse breakdown/withstand voltage `V_R`. Mapping them onto
this model:

- `v_th = V_F` — the point where forward conduction begins.
- `g_on` — pick so that the model's current at your circuit's typical forward current matches
  the datasheet at that same current:

  $$g_{on} \approx \frac{I_{typical}}{V_{typical} - v_{th}}$$

  for a $V_{typical}$ a bit above $V_F$. A real diode's forward curve isn't perfectly linear
  above $V_F$, so this is a local linearization around your operating point, not an exact match
  across the whole curve.
- `v_breakdown = -V_R` (negative, since it's on the reverse side) if you want to model reverse
  breakdown at all; `g_breakdown=0` disables it entirely (an ideal blocking diode in reverse, as
  in the `first-netlist.md` example) — a legitimate simplification when your circuit never
  approaches the real part's breakdown voltage.
- `g_off` is the reverse leakage conductance — usually small enough (`1e-6` or smaller) to be
  circuit-irrelevant; `0` is fine unless you specifically need to model leakage-driven
  discharge of a high-impedance node.

## `kind=ideal_switch`: on-resistance plus body diode

The netlist keyword is `ideal_switch`; the underlying Rust type is `IdealSwitch` — deliberately
not named `Mosfet`. It models a real MOSFET's channel-plus-body-diode behavior as a PWL companion
model (`r_on` when gated on, a PWL body diode when gated off), not real BSIM-style device
physics — `"MOSFET"` is reserved for a future, not-yet-implemented model of that kind
(`crates/pwl-devices/src/ideal_switch.rs`'s own doc comment; see `docs/journal/` for the rename
that introduced the current name).

The real physics it models, confirmed against how a real MOSFET behaves (not assumed):

- **Gated on**: the channel conducts in *either* direction — a real MOSFET channel is a
  resistor, not a one-way valve — so this is just `r_on` between drain and source, regardless of
  the sign of $V_{ds}$. (This is what synchronous rectification exploits: the channel carries
  freewheeling current at a much lower drop than the body diode would, for $V_{ds} < 0$ too, as
  long as the gate is still on.)
- **Gated off**: the channel is open, but the body diode (anode at the source, cathode at the
  drain, for an N-channel device) still conducts if the circuit pushes current backward through
  it — blocking for $V_{ds} > 0$, conducting for $V_{ds} < -V_f$.

So it's not "$V_{ds}>0$ → switch behavior, $V_{ds}<0$ → diode behavior" as two $V_{ds}$-conditioned
regimes; it's "gate on → switch behavior for *any* $V_{ds}$," and "gate off → body-diode-only
behavior for *any* $V_{ds}$." Which regime applies is decided externally by the gate command
(from `dae-runtime`, see [Gate bindings](gate-bindings.md)) before the circuit is even built each
step — never something an LCP resolves as part of the switch's own physics.

Parameters combine the same 5 body-diode fields as `kind=ideal_diode` with one more:

- `r_on` — on-state channel resistance, drain-source, conducting both directions.
- `g_breakdown`, `v_breakdown`, `g_off`, `v_th`, `g_on` — the body diode's own curve, in its
  natural terms (forward conduction for $V(\text{source}) - V(\text{drain}) > v_{th}$).
- `gate=`/`ctrl=` — how the gate state is resolved; see [Gate bindings](gate-bindings.md) for
  the full detail.

## The node-order convention: `(drain, source)` — read this before wiring a switch

**A `kind=ideal_switch` device card declares its two terminals `(drain, source)`, plain
SPICE-conventional order.** This one bit a real user before it was fixed, and is worth not
burying:

An earlier version of `IdealSwitch` required the netlist to declare `(source, drain)` instead,
so the body diode's *unmirrored* curve stamped correctly as-is. That convention was a real,
confirmed footgun: 6 of 8 switches in the TIDA-010954 cycloconverter experiment were declared in
the natural, SPICE-conventional `(drain, source)` order and so had their body diodes backwards.
The struct's contract changed specifically to remove that footgun, not just document it better
— `IdealSwitch::body_diode_for_drain_source_stamping()` now returns the body diode's curve
already mirrored (via `IdealDiode::reversed()`) so that evaluating it with
$V = V(\text{drain}) - V(\text{source})$ (the node order you actually write in the netlist) gives
the physically correct result directly: blocking for $V > 0$, conducting for $V < -v_{th}$. If
you're reading `pwl-devices`
source directly and see `body_diode` vs. `body_diode_for_drain_source_stamping()`, the latter is
the one every caller stamping a netlist-declared switch actually uses — `body_diode` alone is in
the source's own (source, drain) convention, not the netlist's.

Practically: write `D1 drain source idealswitchmodel` the same way you'd write any SPICE MOSFET
— nothing special to remember about swapping terminals — and the body diode orientation comes
out correct automatically.

## Cross-references

- [Gate bindings](gate-bindings.md) covers every `gate=`/`ctrl=` variant for `kind=ideal_switch`
  in full — this chapter only touches it enough to place it in context.
- [Component Reference](component-reference.md#gate-binding) has the exact netlist form and
  error messages for both device kinds.
- [Troubleshooting and gotchas](gotchas.md) lists the node-order trap and the shared-`r_on`
  constraint as standalone, searchable entries.
