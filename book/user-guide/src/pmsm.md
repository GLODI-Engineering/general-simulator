# The PMSM block

`kind=pmsm` is a permanent-magnet synchronous motor in the rotor `d`/`q` frame — the standard
textbook electromechanical model, built specifically to close the loop around the
[coordinate-transform](coordinate-transforms.md) blocks for a field-oriented-control (FOC)
motor-drive experiment. The full field reference lives in the
[Component Reference](component-reference.md#pmsm-permanent-magnet-synchronous-motor); this
chapter is the conceptual tour.

## Fields

```text
NAME kind=pmsm r_s=<f64> l_d=<f64> l_q=<f64> lambda_pm=<f64> pole_pairs=<f64> \
     inertia=<f64> friction=<f64> inputs=<vd,vq,t_load> [outputs=<id,iq,omega_m,theta_e>]
```

- `r_s` — stator resistance, Ω (must be `>= 0`).
- `l_d`, `l_q` — `d`/`q`-axis inductance, H (both must be `> 0`).
- `lambda_pm` — permanent-magnet flux linkage, Wb.
- `pole_pairs` — electrical cycles per mechanical revolution (must be `> 0`).
- `inertia` — rotor (+ load, if lumped in) inertia, kg·m² (must be `> 0`).
- `friction` — viscous friction coefficient, N·m·s/rad (must be `>= 0`).
- `inputs=<vd,vq,t_load>` — exactly three, in this order: rotor-frame stator voltage commands
  `vd`/`vq` (V — typically a `clarkeparkinv`'s output, or wired directly from PID outputs), and
  the mechanical load torque `t_load` (N·m).

## The four outputs

`id`, `iq` (A, stator current in the rotor frame), `omega_m` (mechanical speed, rad/s), and
`theta_e` — the electrical angle, **already wrapped to `[0, 2*pi)`** (via `Pmsm::theta_e_wrapped`
internally — the block integrates an unwrapped angle for its own RK4 stepping, so RK4 never has
to reason about a mid-step discontinuity, and wraps only the output). Feed `theta_e` directly
into a `kind=park`/`kind=clarkepark` block's `theta` input — that's the whole point of wrapping
it for you rather than leaving the caller to do it. `outputs=` defaults to
`<name>,<name>_iq,<name>_omega_m,<name>_theta_e` if left unset, matching the
`kind=clarke`-style convention (see [Coordinate transforms](coordinate-transforms.md)).

The motor starts at rest — $i_d = i_q = \omega_m = \theta_e = 0$ — unless `ic=[id,iq,omega_m,theta_e]`
says otherwise, the same `ic=` spelling every other stateful block and the `C`/`L` electrical
elements use (see [Dynamic blocks](dynamic-blocks.md#ic-starting-a-block-somewhere-other-than-rest)).
Starting a drive already spinning, `ic=[0,0,100,0]`, skips the mechanical run-up that otherwise
dominates the run time of anything studying the electrical behaviour at speed.

## Surface-mount vs. interior-PM

`l_d == l_q` models a surface-mount PMSM: no reluctance torque, the electromagnetic torque comes
purely from the `lambda_pm`/`iq` interaction. `l_d != l_q` (typically `l_d < l_q` for an
interior-PM rotor) adds a genuine reluctance-torque term,

$$(L_d - L_q)\, i_d\, i_q$$

— this is exactly why the block can't be built as an ordinary linear `statespace`: that term is
*bilinear* (a product of two of the block's own states), not linear in the state vector, so it
falls outside the descriptor-DAE shape $Ax + K\dot{x} = Bu$ every other dynamic block in this
project compiles to (see [Dynamic blocks](dynamic-blocks.md) for that shape). `Pmsm` instead carries its
own bespoke `step()`, integrated with RK4 over its full 4-state nonlinear vector field directly —
the same category as `kind=vco`, a small self-contained block rather than a compiled
`StateSpace`. Because the whole nonlinear field is evaluated inside one `step()` call, RK4's own
intermediate stages handle the bilinear coupling correctly without needing any cross-block
algebraic-loop resolution the way splitting `id`/`iq` dynamics across separate blocks would.

## A minimal id=0 FOC speed-loop sketch

The standard cascade — an outer speed loop commanding `iq` (with `id` held at zero, the simplest
FOC strategy for a surface-mount motor with no reluctance torque to exploit), and inner current
loops on `id`/`iq` producing `vd`/`vq` through `clarkeparkinv` — looks schematically like:

```text
* speed error -> PID -> iq reference (id reference fixed at 0)...
SPEED_ERR kind=sum inputs=OMEGA_REF,M1_omega_m signs=1,-1
IQ_REF kind=pid in=SPEED_ERR kp=... ki=... kd=0 n=1000 clamp_lo=-I_MAX clamp_hi=I_MAX
ID_REF kind=const value=0
* current loops (feeding back the motor's own id/iq through prev: -- see the Signals chapter
* for why a same-step reference here would be a genuine algebraic loop)...
ID_ERR kind=sum inputs=ID_REF,prev:M1 signs=1,-1
IQ_ERR kind=sum inputs=IQ_REF,prev:M1_iq signs=1,-1
VD kind=pid in=ID_ERR kp=... ki=... kd=0 n=1000 clamp_lo=-V_MAX clamp_hi=V_MAX
VQ kind=pid in=IQ_ERR kp=... ki=... kd=0 n=1000 clamp_lo=-V_MAX clamp_hi=V_MAX
M1 kind=pmsm r_s=... l_d=... l_q=... lambda_pm=... pole_pairs=... inertia=... friction=... \
     inputs=VD,VQ,TLOAD
```

This is a schematic sketch, not a verified gain set — the fully worked cascade (real current-
and speed-loop gain derivations, the complete `prev:`-closed topology, and a verified plot)
lives in an internal PMSM field-oriented-control (FOC) drive experiment's README, Stage 1;
reproduce that experiment for real numbers rather than guessing gains from this sketch alone.

## Cross-references

- [Coordinate transforms (Clarke/Park) and PLL](coordinate-transforms.md) — `clarkepark`/
  `clarkeparkinv`, which `theta_e` and `vd`/`vq` are built to interoperate with directly.
- [Dynamic blocks](dynamic-blocks.md) — the `kind=pid` current/speed loops above.
- [Signals](signals.md) — `prev:`, needed anywhere a controller regulates the very block whose
  own current-step output would otherwise create an algebraic loop.
