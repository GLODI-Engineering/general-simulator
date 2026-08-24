# `general-simulator-cli`: the netlist-in/CSV-out runner

*(Skeleton — outline below; not yet written.)*

## What goes here
- Why the netlist parser is hand-rolled (no serde/TOML) and deliberately reuses SPICE `*`
  comments as the device/block declaration channel — the "one file, valid to any other tool
  too" design goal, and what it costs (a device line detection heuristic keyed on the literal
  substring `kind=`, which can misfire if that string appears in ordinary prose comments — a
  real trap worth documenting for contributors editing this file's own doc comments).
- How `GateSpec` (CLI-local, string/field parsing) maps to `GateBinding`
  (`dae-runtime`, resolved values) — the translation layer, and why it exists as a separate type
  rather than parsing directly into `GateBinding`.
- Testing strategy: spawning the actual built binary against fixture netlists that are the
  exact circuits already hand-verified elsewhere in the workspace — a cross-check, not a fresh
  derivation, and why that distinction matters for what these tests actually prove.

## Source material to adapt from
- `crates/general-simulator-cli/src/main.rs` module doc comment and the `GateSpec`/`Kind` types.
- `crates/general-simulator-cli/tests/cli.rs` doc comment for the testing-strategy framing.
