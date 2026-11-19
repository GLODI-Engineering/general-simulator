# Installing and building

Three ways in, cheapest first.

## From crates.io

```bash
cargo install general-simulator-cli
```

This builds and installs the `general-simulator` binary from the published crate. It needs a
Rust toolchain and nothing else.

## A prebuilt binary

Every [release](https://github.com/GLODI-Engineering/general-simulator/releases) attaches four
archives — Linux x86-64, Linux ARM64, macOS (Apple silicon) and Windows x86-64 — each holding
the `general-simulator` binary built with default features. Download, unpack, put it on your
`PATH`; no toolchain required.

Both of the above give you the **default** build. If you need `kind=pyblock` or `kind=pyfunc`,
which embed Python, you need the `python` Cargo feature and therefore a build of your own — see
below.

## From source

## Prerequisites

A Rust toolchain (`rustup`/`cargo`). No other system dependency is required for the core build;
two block kinds pull in optional ones only if you enable them:

- `kind=pyblock`/`kind=pyfunc` (embedded Python via PyO3) needs the `python` Cargo feature:
  `cargo build --release --features python -p general-simulator-cli`. Without it, the plain
  build below still works — those two block kinds just aren't available.
- `kind=octfunc`/`kind=octblock` (GNU Octave, via subprocess — never linked, keeping the
  runtime Octave dependency fully decoupled) needs `octave-cli` on `PATH` at *run* time, not
  build time — no Cargo feature gates it.

## Building

```bash
cargo build --release -p general-simulator-cli
```

The binary ends up at `target/release/general-simulator`. Use the release profile for anything
beyond a toy netlist — the LCP solve and the symbolic-expression evaluation it drives are
measurably slower in a debug build. `cargo build` (debug) is fine for iterating on the simulator's
own source, not for running an actual transient.

## Workspace layout, at a glance

```text
crates/
├── lcp-solver          Lemke's algorithm
├── pwl-devices          ideal-diode / ideal-switch PWL models
├── dae-runtime           circuit assembly + the transient loop (the actual engine)
├── continuous-blocks     the block library (PID, state-space, PWM, coordinate transforms, ...)
├── gsim-rules            the rule engine: watches named quantities during a run and reports a
│                          violation as a pair of edge events (kind=rule, see rules-and-ratings.md)
├── cscript-ffi            the native (C ABI) scripting-block escape hatch
├── pyblock-ffi            the embedded-Python scripting-block escape hatch (kind=pyblock/pyfunc)
├── octave-ffi             the subprocess-Octave scripting-block escape hatch (kind=octfunc/octblock)
└── general-simulator-cli  the netlist-in / CSV-out (or SPICE rawfile) runner -- this is what you run
```

`general-simulator` itself only adds PWL device segments (resolved via LCP) and the continuous-
block library; the underlying MNA stamping and netlist parsing are path dependencies on two
sibling repos (`general-mna`, `spice-lsp`) this crate doesn't re-implement. See the Developer
Guide's "Crate tour" for what each crate is actually responsible for.

## Running the test suite

```bash
cargo test --workspace
```

is a reasonable sanity check right after building — it doesn't need `--features python` unless
you're specifically touching `pyblock-ffi` or the scripting-block tests that depend on it. `cargo
test --workspace` does include `crates/octave-ffi`'s own tests, and those genuinely need
`octave-cli` on `PATH` — they call `OctaveSession::spawn()` and `.unwrap()` the result, so a
missing `octave-cli` fails those tests outright rather than skipping them.
