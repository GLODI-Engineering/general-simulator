# Buck converter (open-loop and PID)

*(Skeleton — outline below; not yet written.)*

## What goes here
- Open-loop buck: topology, `gate=pwm`, expected `Vout = duty * Vin`, one plot.
- Closed-loop buck: adding the `sum`/`pid`/`gate=dutyctrl` chain, gain derivation shown, one
  plot with reference tracking.
- A short "what can go wrong" callout referencing the underdamped-resonance and hysteresis
  variants as further reading, not repeated here.

## Source material to adapt from
- `internal-archive/experiments/elspice-pwl-buck-vs-xyce-ngspice/`,
  `elspice-pwl-buck-underdamped-resonance-filter/`,
  `elspice-pwl-buck-hysteresis-current-mode/`, and `elspice-pwl-buck-dc-motor-cascade/` — pick
  one or two representative ones for this chapter; link to the rest rather than reproducing
  every variant in the book.
