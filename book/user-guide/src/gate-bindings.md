# Gate bindings (fixed, PWM, block-driven)

**A note on this chapter's own title, before anything else**: earlier project history (and this
chapter's own outline) described several `gate=` variants — `on`/`off`, `pwm`, `dutyctrl`,
`dutyctrlcomplement`, `vco`, `block`, `vcophase`. That's no longer how it works. `GateBinding`
today has exactly one variant and exactly one job: reading a named block's current output and
thresholding it at `>= 0.5` (`general-mna/src/block_graph.rs`'s `GateBinding` doc comment).
**There is no non-block-driven gate at all** — even a permanently-on or permanently-off switch
is an ordinary block wired the same way a PWM-driven one is. What used to be several distinct
`gate=` spellings is now one mechanism applied consistently; this chapter documents the current
one, not the historical variant list.

## The one form

```text
NAME kind=ideal_switch ... gate=block ctrl=<sig2phys_voltage_block_name>
```

- `gate=block` — the only accepted value.
- `ctrl=<name>` — the name of a declared `domain=voltage` [`kind=sig2phys`](component-reference.md#sig2phys-signal-to-physical-converter)
  block. Every step, the switch reads that block's current output and is on while it's `>= 0.5`.

Why it has to be a `sig2phys` converter specifically, rather than a raw `pid`/`pwm`/`hysteresis`
block: a circuit quantity and a signal-domain value are treated as genuinely different kinds of
thing throughout this grammar (see [Grammar overview](netlist-grammar.md) and
[Component Reference: Sig2Phys](component-reference.md#sig2phys-signal-to-physical-converter)) —
an ideal switch's gate is itself a voltage ($V_{GS}$ against $v_{th}$), not a distinct
discrete-actuation signal domain, so it shares the same write-direction converter a `V`-source's
own magnitude uses, not a dedicated gate-only type.

## A "fixed" gate is just a `const` wired through `sig2phys`

Since there's no dedicated on/off spelling, a permanently-on or permanently-off gate is built
from the same two blocks as any other:

```text
ONVAL kind=const value=1
ONGATE kind=sig2phys domain=voltage in=ONVAL
V1 in 0 5
D1 in out idealswitchmodel
D1 kind=ideal_switch r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.5 g_on=5 \
     gate=block ctrl=ONGATE
R1 out 0 1k
```

Verified end to end (this is the Component Reference's own [Gate Binding
example](component-reference.md#gate-binding)): `V(out)` settles to

$$V(\text{out}) = \frac{5 \times 1000}{1000 + 0.1} \approx 4.9995$$

— a fully-on 0.1 Ω switch in series with a 1 kΩ load.

## A PWM-driven gate: half-bridge pattern

`kind=pwm` and `kind=pspwm` (see [Component Reference](component-reference.md#pwm-modulator-1))
are both **active-high complementary**: two outputs, `main` and `complement`, with `complement`
the exact logical NOT of `main` — never an independently-computed inverted signal. Driving a
half-bridge leg's two switches means wrapping each output in its own `sig2phys` converter and
wiring each to its own switch's gate:

```text
DUTY kind=const value=0.3
LEG kind=pwm freq=10000 in=DUTY outputs=LEG_MAIN,LEG_COMP
LEG_MAIN_G kind=sig2phys domain=voltage in=LEG_MAIN
LEG_COMP_G kind=sig2phys domain=voltage in=LEG_COMP
* high-side switch: gate=block ctrl=LEG_MAIN_G
* low-side switch:  gate=block ctrl=LEG_COMP_G
```

This is why the pattern guarantees no shoot-through and no gap by construction, not by careful
timing: both gates come from the *same* carrier and the *same* duty command, related by an exact
logical complement, not two independently-tuned PWM instances that happen to usually agree. Dead
time (`red=`/`fed=`, seconds) delays only the two *rising* (turn-on) edges — never a falling
edge — which is what makes both outputs *provably* low during the gap: whichever switch was
conducting always turns off exactly on schedule, and only the other one is held off a little
longer before it's allowed to turn on. `red=fed=0.0` (the default) recovers the ideal,
gap-free, overlap-free pair exactly. A topology with only one actively-driven switch (a buck
converter's high-side, freewheeling through a diode) just leaves `complement` unwired.

`kind=pspwm` (Phase-Shift PWM) is the frequency-and-phase-driven variant — used for
frequency-modulated topologies like an LLC converter, where the switching frequency itself is
the control variable rather than duty. It owns its own internal oscillator state directly (not a
`kind=vco` wired in) specifically so that two `pspwm` instances fed the *same* `freq` input stay
bit-for-bit phase-synchronized — the way a bridge's two legs need to be — without a
separately-declared shared oscillator block in between.

### `pspwm`'s `phase` input is a lead, not a lag

The block keeps its own integrated carrier phase $\varphi \in [0, 1)$ (cycles), advanced each
step by the clamped frequency command, and gates on

$$\theta = (\varphi + \text{phase}) \bmod 1,$$

with `main` on for $\theta \in [\text{red}, \text{duty})$. Adding the `phase` input *advances*
$\theta$, so a positive `phase` command moves every edge **earlier**: the on-interval of a block
commanded `phase` $= p$ begins at

$$t_{\text{on}} = (1 - p)\,T \pmod T,$$

where $T = 1/f$ is the carrier period — not at $p\,T$. A phase of $0.25$ starts the pulse three
quarters of the way through the reference block's period, not one quarter.

Concretely, at $f = 1\,\text{Hz}$, 50 % duty and $\Delta t = 0.125\,\text{s}$:

```text
FREQ kind=const value=1
PH0  kind=const value=0
PH25 kind=const value=0.25
DUTY kind=const value=0.5
REF  kind=pspwm f_min=0.1 f_max=10 inputs=FREQ,PH0,DUTY
LEAD kind=pspwm f_min=0.1 f_max=10 inputs=FREQ,PH25,DUTY
```

`REF` first rises (its carrier wraps) at $t = 1.0\,\text{s}$; `LEAD` rises at
$t = 0.75\,\text{s}$ — a quarter period *before* it, and its falling edge at $0.25\,\text{s}$
likewise precedes `REF`'s at $0.5\,\text{s}$. `doc-verify/pspwm/phase_lead.cir` reproduces
this run and `test_pspwm.py::test_phase_input_is_a_lead_not_a_lag` asserts it.

Why it is easy to get wrong: for a single modulator the sign is invisible — the waveform is a
correct square wave at the right duty and frequency, merely positioned elsewhere. It only
matters once two modulators must hold a phase relationship, which is what the block exists
for. In a symmetric structure it can even partially cancel: mirroring both legs of a bridge
about the period preserves the *difference* waveform when the inner angle is the only shift,
so a dual-active-bridge primary can come out right while the secondary, which carries the main
phase shift, does not. If you want a leg to start its pulse at a fraction $s$ *into* the
reference period (a lag), command `phase` $= 1 - s$.

## Two real errors worth knowing before you hit them

**`ctrl=` naming something that isn't a `domain=voltage` sig2phys.** Wiring `ctrl=` directly to
a `pid`/`vco`/`hysteresis` block, or to a `domain=current` sig2phys, is rejected at
`dae-runtime`'s validation stage (not netlist parse time): `DaeError::GateTargetNotSig2Voltage`.
The fix is always the same: put a `kind=sig2phys domain=voltage in=<your block>` in between.

**Using a converter's name as an ordinary circuit node.** A `kind=sig2phys` block has no
terminals and stamps nothing into the MNA system — it can only ever be *referenced by name*
(a source's value field, or `ctrl=`), never wired as a node. `general-mna` itself can't detect
this mistake (it never sees the block-graph half of the picture when building the electrical
system alone), so writing e.g. `R1 VDRV 0 1k` where `VDRV` happens to also be a sig2phys
converter's name used to build and solve without complaint — the net was simply undriven, and
KCL silently solved it to $V(\text{VDRV}) = 0$, right next to that same-named block's own correct,
nonzero output column. This is now a hard, up-front error in both `--mode dc` and `--mode
transient`: `DaeError::Sig2PhysUsedAsCircuitNode`, naming the converter, the offending element,
and the node token as written. See `doc-verify/sig2phys/error_converter_wired_as_a_node.cir` for
a real fixture reproducing it.

## Cross-reference

This chapter is about how a gate's *state* gets decided. How a block's own *inputs* are wired —
`prev:`, same-step references, the `AlgebraicLoop` error — is a related but different question,
covered in [Signals](signals.md).
