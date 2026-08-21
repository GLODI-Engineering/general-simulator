# DAE integration: backward Euler and trapezoidal

*(Skeleton — outline below; not yet written.)*

## What goes here
- The descriptor DAE shape used throughout: $A x + K \dot{x} = B u$.
- Backward Euler derivation: $\dot x \approx (x_{n+1}-x_n)/dt \Rightarrow (A + K/dt)\,x_{n+1} =
  B u_{n+1} + (K/dt)\,x_n$ — first-order, tolerant of an inconsistent `x_prev`.
- Trapezoidal derivation: sum the DAE at $t_n$ and $t_{n+1}$, eliminate the derivative via
  $\dot x_n + \dot x_{n+1} = (2/dt)(x_{n+1}-x_n)$, arrive at
  $(A/2 + K/dt)\,x_{n+1} = (B/2)(u_n+u_{n+1}) + (K/dt - A/2)\,x_n$ — second-order, but needs
  $x_n$ already consistent.
- Why trapezoidal needs consistent initial conditions (the "DAE analog" framing), and therefore
  why every run starts with backward Euler and falls back to it after any resolved-segment or
  gate-state change.
- The `coupling_scale = 0.5` detail for trapezoidal's diode-current term, and the transient
  time-varying-source (`SIN`/`PWL`/...) averaging this required once those were added.

## Source material to adapt from
- `crates/dae-runtime/src/lib.rs`'s `Scheme` enum doc comments (`Dc`/`BackwardEuler`/
  `Trapezoidal`) — the full derivation is already there, including the transient-source
  addendum; port with KaTeX formatting.
