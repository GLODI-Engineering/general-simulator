# Boost/LLC resonant converter

*(Skeleton — outline below; not yet written.)*

## What goes here
- Boost topology basics, then the LLC resonant case specifically: why it's frequency-modulated
  rather than duty-modulated (`gate=vco`, not `dutyctrl`), and the `sum -> pid -> gain -> vco`
  chain this implies.
- The half-bridge complementary-switch pattern in a real topology (cross-reference
  `gate-bindings.md` instead of re-explaining it).
- One plot: closed-loop frequency regulation settling to a reference.

## Source material to adapt from
- `internal-archive/experiments/elspice-pwl-boost-llc-vs-xyce-ngspice/` and
  `elspice-pwl-llc-closed-loop-vs-xyce-ngspice/`.
