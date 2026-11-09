# Gotchas index

Manually maintained for now (no `scripts/gotchas-index.sh` automation yet, unlike
`general-mna`).

| ID | Title | Severity | Status | Scope | Discovered |
|---|---|---|---|---|---|
| [GOTCHA-001](GOTCHA-001-cscript-ffi-fixture-race.md) | `cargo test -p cscript-ffi` intermittently fails with `dlopen failed` | low | fixed | `crates/cscript-ffi/tests/**`, `crates/general-simulator-cli/tests/**` | 2026-08-29 |
| [GOTCHA-002](GOTCHA-002-phys2sig-one-step-sampling-delay.md) | A `phys2sig` column is one step behind the quantity it reads, and a steady-state check cannot see it | medium | documented | `crates/dae-runtime/src/block_graph.rs`, `doc-verify/phys2sig/**` | 2026-11-06 |
| [GOTCHA-003](GOTCHA-003-capital-m-is-milli-not-mega.md) | `1M` is one milliohm, not one megohm | high | documented | `crates/general-simulator-cli/**`, `book/user-guide/src/netlist-grammar.md` | 2026-11-09 |
