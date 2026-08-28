# Gotchas index

Manually maintained for now (no `scripts/gotchas-index.sh` automation yet, unlike
`elspice-mna`).

| ID | Title | Severity | Status | Scope | Discovered |
|---|---|---|---|---|---|
| [GOTCHA-001](GOTCHA-001-cscript-ffi-fixture-race.md) | `cargo test -p cscript-ffi` intermittently fails with `dlopen failed` | low | open | `crates/cscript-ffi/tests/**` | 2026-08-29 |
