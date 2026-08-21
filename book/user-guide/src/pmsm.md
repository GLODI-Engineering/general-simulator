# The PMSM block

*(Skeleton — outline below; not yet written.)*

## What goes here
- `kind=pmsm` field reference (`r_s`, `l_d`, `l_q`, `lambda_pm`, `pole_pairs`, `inertia`,
  `friction`, `inputs=<vd>,<vq>,<t_load>`).
- The four outputs (`id`, `iq`, `omega_m`, `theta_e` — already wrapped to `[0, 2*pi)`) and the
  direct "feed `theta_e` into a `park`/`clarkepark` block" pattern.
- Surface-mount (`l_d == l_q`) vs. interior-PM (reluctance torque) — one line each, when to use
  which.
- A minimal id=0 FOC speed-loop recipe, pointing at the PMSM drive experiment's Stage 1 for the
  full worked, verified version (including the `prev:`-closed cascade) rather than repeating it.

## Source material to adapt from
- `crates/continuous-blocks/src/pmsm.rs` module doc comment.
- `internal-archive/experiments/elspice-pwl-pmsm-foc-drive/README.md` for the worked
  cascade design (current-loop and speed-loop gain derivations, with real numbers).
