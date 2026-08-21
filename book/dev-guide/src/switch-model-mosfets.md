# MOSFET channel state: the exogenous case, and why

*(Skeleton — outline below; not yet written — but the narrative content already exists almost
verbatim in the journal; see below.)*

## What goes here
- Gate state is never resolved by the LCP, and never modeled as a `Vgs` vs. `Vth` comparison
  anywhere in the crate — it's an external, exogenously-known control input, decided before the
  circuit is even built. Quote `pwl_devices::Mosfet`'s own doc comment directly.
- The two structural stamps a known gate state picks between: **on** -> a plain linear switch
  (`r_on`, no complementarity variable at all), **off** -> channel opens, body diode folds into
  the diode LCP (previous chapter).
- **The math reason this can't just join the diode LCP**: a diode's segments share one
  reference conductance, keeping $A_0$ fixed regardless of `z` — a switch's on/off is a
  genuine *topological* change to $A_0$ itself (a conductance path appearing/disappearing, not
  a current-source shift at fixed topology), which breaks the fixed-$A_0$ assumption the whole
  LCP fold depends on. This is why `build_with_mosfets` calls `MnaBuilder` with different
  `BuildOptions` and rebuilds the *symbolic* system every step, rather than reusing one $A_0$.
- **The physical reason it's exogenous anyway, independent of the math**: in every circuit this
  crate targets (PWM power converters), the gate signal genuinely *is* an external control
  input in the real system too — a real gate driver doesn't discover it should switch by
  solving a complementarity condition, a controller commands it. Treating it as exogenous is
  model fidelity, not a shortcut.
- What *would* be a legitimate different feature: a genuinely self-triggering ideal switch
  (relay, fuse, natural-commutation-only MOSFET) — still not "the same LCP as diodes" for the
  reason above, but a real, distinct, currently-unimplemented extension. Forward-reference
  `open-questions.md`.

## Source material to adapt from
- **`docs/journal/2026-08.md`, entry "Robustness Q&A..." (2026-08-21), Q4 and Q5** — Q5
  specifically answers "is it not better to bring MOSFET switching into the LCP/DAE scheme
  too?" with the full argument already reasoned through; this chapter is largely that entry
  expanded with the KaTeX formulas and the worked example.
- `crates/pwl-devices/src/mosfet.rs` module doc comment.
- `crates/dae-runtime/src/lib.rs`'s `build_with_mosfets` and its doc comment.
