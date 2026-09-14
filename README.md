# general-simulator

`general-simulator` is an educational Rust simulator for piecewise-linear (PWL) circuits and
mixed circuit/block-diagram systems that never runs Newton-Raphson on device physics, and
therefore never needs SPICE-style voltage limiting.

## Why

SPICE-family simulators (ngspice, Xyce) solve nonlinear device equations with Newton-Raphson,
which needs "voltage limiting" to converge on exponential diode/transistor curves. Voltage
limiting works, but is inconsistent, path-dependent (hysteretic under backtracking), and
incompatible with most modern nonlinear-solver enhancements — a known wart even by its own
maintainers' account (see Sandia's own PCNR paper, Aadithya/Keiter/Mei 2020, which proposes a
partial fix). Full background and the source documents are kept in an internal reference
archive maintained alongside this project.

This project sidesteps the problem instead of patching it: every active device is modeled as
piecewise-linear (a diode's three conduction segments, an ideal switch's controlled/natural
commutation modes), so within any *fixed* combination of active segments the whole circuit is
exactly linear. Which segment each device is in, per timestep, is decided by solving a **Linear
Complementarity Problem (LCP)** via Lemke's algorithm — the same rigorous mode-selection
approach commercial power-electronics simulators use — instead of continuous Newton iteration. See
[`docs/architecture.md`](docs/architecture.md) for the full formulation.

## Relationship to the sibling repos

Path deps, read-only, same convention `general-mna` already uses for `general-spice-core`:

```
Électronique/
├── general-simulator/        (this repo)
├── general-mna/         symbolic MNA + converter averaging
└── spice-lsp/            general-spice-core: the ngspice/Xyce parser
```

- `general-spice-core` remains the sole netlist-parsing authority.
- `general-mna` remains the sole implementation of *linear*-device MNA stamping
  (R, C, L, V, I, G, E, F, H) and numeric Schur-complement reduction — this repo reuses those
  directly rather than re-deriving them.
- `general-simulator` adds exactly two new things: PWL device segments resolved via LCP, and a
  continuous-block library (transfer function, state-space, PID, ...) compiled into descriptor
  DAE fragments of the same `A x + K dx/dt = B u` shape `general-mna` already uses.

## Status

All six original milestones are implemented, each verified against independently hand-derived
results before anything built on top of it (see `docs/journal/` for the full account of each,
including bugs caught and fixed along the way):

- `crates/lcp-solver` — Lemke's algorithm, verified against hand-solved textbook LCP fixtures.
- `crates/pwl-devices` — a 3-segment PWL ideal diode (`IdealDiode`) and an ideal switch
  (`IdealSwitch`, gated-on switch / gated-off body diode), verified against hand-derived circuit
  operating points. Named "ideal switch" rather than "MOSFET" because that name is reserved for
  a future, not-yet-implemented BSIM-style device model (see `docs/journal/` for the rename).
- `crates/dae-runtime` — folds any netlist's linear part plus PWL ideal diodes/ideal switches
  into one LCP, with a full transient timestep loop (trapezoidal, backward Euler on the first
  step and any LCP-resolved mode change), ideal-switch/PWM support, and closed-loop wiring for a
  `continuous-blocks` controller (e.g. a PID) driving a switching gate.
- `crates/continuous-blocks` — transfer function / state-space / PID / integrator / math-op
  compilation into the same descriptor-DAE shape circuits use, standalone and verified.
- `crates/general-simulator-cli` (binary `general-simulator`) — netlist-in/CSV-waveform-out runner.

Open: validation against the Xyce/ngspice baselines already captured in an internal
validation-experiment archive maintained alongside this project, full converter benchmarks, and
the smaller scope notes recorded in each milestone's own journal entry (per-instance ideal-switch
`Ron`, non-diode-only transient variants, etc.).

## Development

```bash
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

Continuity log: [`docs/journal/`](docs/journal/). Read the newest entry before continuing prior
work. Non-obvious traps: [`docs/gotchas/INDEX.md`](docs/gotchas/INDEX.md).
