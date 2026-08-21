# FAQ

*(Skeleton — outline below; not yet written.)*

## What goes here
- "Why not just use ngspice/Xyce?" — two-line answer plus a link to the dev guide's full
  architecture rationale.
- "Can I model a switch that turns on/off based on its own circuit state (not PWM-driven)?" —
  short "not directly today" answer; link to the dev guide's open-questions chapter.
- "Why do I need `prev:` here instead of just wiring it directly?" — link to `signals.md`.
- "My duty/gate command is way outside [0,1] and the loop won't converge" — link to the PID
  dynamic-clamp gotcha and the PFC worked example's own debugging account, since this is a real
  failure mode a new user doing closed-loop power-electronics control is likely to hit.
- Populate the rest of this from actual questions once early users start asking them — resist
  the urge to invent hypothetical FAQ entries no one has asked yet.

## Source material to adapt from
- None yet — this chapter should grow from real user questions after publication, not be
  fully pre-written.
