# general-simulator

[![CI](https://github.com/GLODI-Engineering/general-simulator/actions/workflows/ci.yml/badge.svg)](https://github.com/GLODI-Engineering/general-simulator/actions/workflows/ci.yml)
[![crates.io](https://img.shields.io/crates/v/general-simulator-cli.svg)](https://crates.io/crates/general-simulator-cli)
[![docs.rs](https://img.shields.io/docsrs/dae-runtime)](https://docs.rs/dae-runtime)
[![license: AGPL-3.0](https://img.shields.io/badge/license-AGPL--3.0-blue.svg)](LICENSE)

`general-simulator` is a Rust simulator for piecewise-linear (PWL) circuits and mixed
circuit/block-diagram systems that never runs Newton-Raphson on device physics, and therefore
never needs SPICE-style voltage limiting. Since 0.6.0 it also carries a rule engine, so a run
that breaks a rating, a bound or a balance says so instead of returning a plausible waveform.

## Quick start

```bash
cargo install general-simulator-cli
```

Or take a prebuilt binary — Linux x86-64, Linux ARM64, macOS (Apple silicon), Windows x86-64 —
from the [latest release](https://github.com/GLODI-Engineering/general-simulator/releases/latest),
which is also where each version's notes live. To build from the repository instead:

```bash
cargo build --release -p general-simulator-cli
```

The binary lands at `target/release/general-simulator`. Run a deck:

```bash
./target/release/general-simulator deck.cir --mode transient --tfinal 20e-3 --dt 10e-9 > out.csv
```

CSV goes to stdout (`--out` is for `--format raw`); the first row is the column names, one per
circuit unknown and per declared block. See
[Reading the output](book/user-guide/src/reading-output.md).

### If you build with `--features python`

The default build depends on nothing but a Rust toolchain, and the binaries attached to each
[release](https://github.com/GLODI-Engineering/general-simulator/releases) are default builds —
download one and it runs.

`kind=pyblock` and `kind=pyfunc` are the exception. They need PyO3's embedded interpreter, so
they need a `--features python` build:

```bash
cargo build --release -p general-simulator-cli --features python
```

That build links against **one specific `libpython`**, and on most systems it is not on the
default loader path. The failure is early and total, on *any* deck, `pyblock` or not:

```
$ ./target/release/general-simulator deck.cir
./target/release/general-simulator: error while loading shared libraries:
libpython3.12.so.1.0: cannot open shared object file: No such file or directory
$ echo $?
127
```

Point the loader at a Python that provides the **shared** library:

```bash
export LD_LIBRARY_PATH=/path/to/python3.12/lib      # the directory holding libpython3.12.so.1.0
```

Note that a distribution's own `python3.12` often ships only the *static* library, so
`sysconfig`'s `LIBDIR` is frequently the wrong answer. On Debian/Ubuntu the shared one comes from
the `libpython3.12` package; a `uv`- or `pyenv`-managed interpreter built with `--enable-shared`
has it under its own `lib/`. To find it:

```bash
ldd target/release/general-simulator | grep python   # confirms what is missing
find / -name 'libpython3.12.so.1.0' 2>/dev/null     # and where it is
```

This is why the release workflow builds the default feature set: a `--features python` binary is
tied to one libpython, which makes it a worse download than it is a build. See
[The Python escape hatch](book/user-guide/src/python-blocks.md).

### Documentation

The user guide in [`book/`](book/) is the reference: the
[netlist grammar](book/user-guide/src/netlist-grammar.md), every `kind=` block in the
[Component Reference](book/user-guide/src/component-reference.md),
[PWL devices](book/user-guide/src/pwl-devices.md),
[command-line flags](book/user-guide/src/cli-reference.md),
[fixed vs. adaptive stepping](book/user-guide/src/time-stepping.md) and
[troubleshooting](book/user-guide/src/gotchas.md).

To draw a schematic rather than write a netlist, see the sibling
[`gsim-tools`](https://github.com/GLODI-Engineering/gsim-tools) (Python) and the VS Code editor;
the CLI itself takes a `.cir` and nothing else.

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
├── spice-lsp/            general-spice-core: the ngspice/Xyce parser
└── gs-waveform-measurements/  the measurement primitives `kind=measure` evaluates
```

- `general-spice-core` remains the sole netlist-parsing authority.
- `general-mna` remains the sole implementation of *linear*-device MNA stamping
  (R, C, L, V, I, G, E, F, H) and numeric Schur-complement reduction — this repo reuses those
  directly rather than re-deriving them.
- `general-simulator` adds three things: PWL device segments resolved via LCP, a
  continuous-block library (transfer function, state-space, PID, ...) compiled into descriptor
  DAE fragments of the same `A x + K dx/dt = B u` shape `general-mna` already uses, and the rule
  engine that watches a run (`crates/gsim-rules`).

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
- `crates/gsim-rules` — the rule engine: a rule watches a named quantity, a condition decides
  when it is violated, and each violation is reported as a pair of edge events — one when it
  starts, one when it ends, the latter carrying the duration, the peak and when the peak
  happened. It observes only; `action=stop` ending a run is the single exception. Built and
  tested with no solver at all, reading quantities through a trait the caller implements.
- `crates/general-simulator-cli` (binary `general-simulator`) — netlist-in/CSV-waveform-out
  runner, which also strips `kind=measure` lines and writes the event stream (`--events`).

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
