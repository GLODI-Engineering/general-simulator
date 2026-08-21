# Sources and math operations

*(Skeleton — outline below; not yet written.)*

## What goes here
- Source blocks: `const`, `time`, `pwl` — with the "how do I build `sin(2*pi*f*t)`" recipe
  (`time -> gain -> sin`) spelled out once, since it recurs constantly in examples.
- Stateless math ops: `sum`, `gain`, `product`, `saturation`, `table`.
- The full waveform-arithmetic function reference (`cos`/`sin`/`tan`/`exp`/`ln`/... , `atan2`/
  `hypot`/`pow`/`min`/`max`/`angle_wrapped`, `if`/`limit`) as one compact table with arities.
- What's deliberately *not* here and why (no derivative block, no noise/random — link to the
  dev guide's rationale rather than re-arguing it).

## Source material to adapt from
- `crates/continuous-blocks/src/math_ops.rs` and `waveform_arithmetic.rs` doc comments — the
  "not included, on purpose" list in `waveform_arithmetic.rs` is worth quoting close to
  verbatim.
