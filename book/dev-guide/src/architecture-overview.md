# Why not Newton-Raphson

*(Skeleton — outline below; not yet written.)*

## What goes here
- The problem with SPICE-family Newton-Raphson: $J(x_k)\,\Delta x = -g(x_k)$, and why device
  equations like a diode's $I = I_S(e^{V/V_T}-1)$ need "voltage limiting" to converge, and why
  that limiting breaks the clean $g(x)=0$ abstraction (iteration-history-dependent, hysteretic
  under backtracking).
- This project's premise instead: every device piecewise-linear, so the circuit is exactly
  linear *within* any fixed combination of active segments — replace "iterate Newton on
  continuous device physics" with "resolve, once per timestep, which discrete combination of
  segments is active."
- Forward-reference the LCP chapter for the actual mechanism; this chapter is motivation only.

## Source material to adapt from
- `docs/architecture.md`'s "Why not Newton-Raphson" section — near-complete already, port with
  light editing.
- `internal-archive/explanations/xyce/newton-raphson-formulation.md` and
  `voltage-limiting-current-status.md` for the primary-source depth, cited rather than
  re-derived.
