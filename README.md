# elspice-pwl

`elspice-pwl` is an educational Rust simulator for piecewise-linear (PWL) circuits and
mixed circuit/block-diagram systems that never runs Newton-Raphson on device physics, and
therefore never needs SPICE-style voltage limiting.

## Why

SPICE-family simulators (ngspice, Xyce) solve nonlinear device equations with Newton-Raphson,
which needs "voltage limiting" to converge on exponential diode/transistor curves. Voltage
limiting works, but is inconsistent, path-dependent (hysteretic under backtracking), and
incompatible with most modern nonlinear-solver enhancements — a known wart even by its own
maintainers' account (see Sandia's own PCNR paper, Aadithya/Keiter/Mei 2020, which proposes a
partial fix). Full background and the source documents are in the companion
`internal-archive` repo, `explanations/xyce/`.

This project sidesteps the problem instead of patching it: every active device is modeled as
piecewise-linear (a diode's three conduction segments, a MOSFET's controlled/natural
commutation modes), so within any *fixed* combination of active segments the whole circuit is
exactly linear. Which segment each device is in, per timestep, is decided by solving a **Linear
Complementarity Problem (LCP)** via Lemke's algorithm — the same rigorous mode-selection
approach used by a reference tool — instead of continuous Newton iteration. See
[`docs/architecture.md`](docs/architecture.md) for the full formulation.

## Relationship to the sibling repos

Path deps, read-only, same convention `elspice-mna` already uses for `spice-core`:

```
Électronique/
├── elspice-pwl/        (this repo)
├── elspice-mna/         symbolic MNA + converter averaging
└── spice-lsp/            spice-core: the ngspice/Xyce parser
```

- `spice-core` remains the sole netlist-parsing authority.
- `elspice-mna` remains the sole implementation of *linear*-device MNA stamping
  (R, C, L, V, I, G, E, F, H) and numeric Schur-complement reduction — this repo reuses those
  directly rather than re-deriving them.
- `elspice-pwl` adds exactly two new things: PWL device segments resolved via LCP, and a
  continuous-block library (transfer function, state-space, PID, ...) compiled into descriptor
  DAE fragments of the same `A x + K dx/dt = B u` shape `elspice-mna` already uses.

## Status

All six original milestones are implemented, each verified against independently hand-derived
results before anything built on top of it (see `docs/journal/` for the full account of each,
including bugs caught and fixed along the way):

- `crates/lcp-solver` — Lemke's algorithm, verified against hand-solved textbook LCP fixtures.
- `crates/pwl-devices` — a 3-segment PWL diode and a MOSFET (gated-on switch / gated-off body
  diode), verified against hand-derived circuit operating points.
- `crates/dae-runtime` — folds any netlist's linear part plus PWL diodes/MOSFETs into one LCP,
  with a full transient timestep loop (trapezoidal, backward Euler on the first step and any
  LCP-resolved mode change), MOSFET/PWM support, and closed-loop wiring for a
  `continuous-blocks` controller (e.g. a PID) driving a switching gate.
- `crates/continuous-blocks` — transfer function / state-space / PID / integrator / math-op
  compilation into the same descriptor-DAE shape circuits use, standalone and verified.
- `crates/elspice-pwl-cli` (binary `elspice-pwl`) — netlist-in/CSV-waveform-out runner.

Open: validation against the Xyce/ngspice baselines already captured in the sibling
`internal-archive` repo's `experiments/` folder, full converter benchmarks, and the
smaller scope notes recorded in each milestone's own journal entry (per-instance MOSFET `Ron`,
non-diode-only transient variants, etc.).

## Development

```bash
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

Continuity log: [`docs/journal/`](docs/journal/). Read the newest entry before continuing prior
work. Non-obvious traps: [`docs/gotchas/INDEX.md`](docs/gotchas/INDEX.md).
