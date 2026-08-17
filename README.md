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

Milestone 1 in progress: `crates/lcp-solver` implements Lemke's algorithm standalone, verified
against hand-solved textbook LCP fixtures (`crates/lcp-solver/tests/fixtures.rs`) — no circuit
code depends on it yet, by design (see `docs/architecture.md` for the milestone sequencing
rationale: the LCP solver is the highest-risk, most unfamiliar piece and is built and trusted
in isolation first).

## Development

```bash
cargo test --workspace
cargo clippy --all-targets -- -D warnings
cargo fmt --all -- --check
```

Continuity log: [`docs/journal/`](docs/journal/). Read the newest entry before continuing prior
work. Non-obvious traps: [`docs/gotchas/INDEX.md`](docs/gotchas/INDEX.md).
