# Coordinate transforms (Clarke/Park) and PLL

*(Skeleton — outline below; not yet written.)*

## What goes here
- The six transforms (`clarke`, `clarkeinv`, `park`, `parkinv`, `clarkepark`,
  `clarkeparkinv`): field reference (`inputs=`, ordered per transform), and the
  `outputs=`/default-naming convention (primary output aliases the block's own name; the rest
  get `<name>_<suffix>` unless named explicitly).
- `angle_wrapped` (the PLL angle-tracking half): what it computes, and that it's an
  *instantaneous* algebraic function, not a filtered/tracking PLL — set expectations correctly
  (no settling transient to plot, by design).
- A minimal worked SRF-PLL recipe: `clarke -> angle_wrapped -> park`, feeding a `pid` on `q`
  back into a tracked angle — reference the PFC experiment's own Stage 1 for the full worked,
  verified version rather than re-deriving it here.
- Amplitude convention called out explicitly: the standard "2/3" (non-power-invariant) scaling.

## Source material to adapt from
- `crates/continuous-blocks/src/coordinate_transforms.rs` module doc comment — already written
  at close to this level.
- `internal-archive/experiments/elspice-pwl-pfc-three-phase-vsc/README.md`'s Stage 1
  Part A for the worked, verified PLL example (with real numbers and a plot to reference/embed).
