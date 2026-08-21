# `continuous-blocks`: the block library

*(Skeleton — outline below; not yet written.)*

## What goes here
- Why this crate is deliberately standalone (no dependency on `elspice-mna`/`pwl-devices`/
  `dae-runtime`) and what that buys: every block verified against a hand-derived result on its
  own before anything wires it into a whole circuit's global system.
- Module map: `state_space.rs`/`transfer_function.rs`/`pid.rs`/`dynamics.rs` (the
  descriptor-DAE-compiling blocks), `vco.rs`/`hysteresis.rs`/`pmsm.rs` (the bespoke-`step()`
  blocks — link to why each isn't a `StateSpace`), `math_ops.rs`/`waveform_arithmetic.rs`
  (stateless), `coordinate_transforms.rs`.
- The `CoordinateTransform` design specifically: why it's a dispatch enum
  (`MathFn1`/`MathFn2`-style) rather than one `BlockKind` variant per transform, and why
  `angle_wrapped` lives in `MathFn2` instead of becoming a seventh `CoordinateTransform`
  variant (single-output vs. multi-output as the deciding factor).

## Source material to adapt from
- `crates/continuous-blocks/src/lib.rs` module doc comment.
- `crates/continuous-blocks/src/coordinate_transforms.rs` module doc comment.
- `elspice-pwl`'s journal entries for `CoordinateTransform`/`Pmsm`/`angle_wrapped` (the design
  reasoning for each was recorded there as it was built).
