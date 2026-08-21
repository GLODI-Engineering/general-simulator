# Introduction

*(Skeleton — outline below; not yet written.)*

## What goes here
- What `elspice-pwl` is: an educational Rust simulator for piecewise-linear circuits and
  mixed circuit/block-diagram systems.
- The one-paragraph pitch: no Newton-Raphson, no voltage limiting — every device segment is
  piecewise-linear, and which segment is active is resolved once per timestep via a Linear
  Complementarity Problem (LCP).
- Who this guide is for (users wiring up netlists/experiments) vs. the companion Developer
  Guide (contributors, or anyone who wants the math and the "why").
- A two-line "why would I use this instead of ngspice/Xyce" — link to the Developer Guide's
  architecture chapter for the full rationale, don't re-derive it here.

## Source material to adapt from
- Repo root `README.md` — "Why" and "Status" sections, already written at the right level for
  this page.
- `docs/architecture.md`'s opening paragraph (condense, don't re-derive the Newton-Raphson
  argument here — that belongs in the dev guide).
