# Installing and building

*(Skeleton — outline below; not yet written.)*

## What goes here
- Prerequisites: Rust toolchain version, `cargo build --release -p general-simulator-cli`.
- Where the binary ends up (`target/release/general-simulator`), and a note about using the release
  profile for anything beyond a toy netlist (debug builds are noticeably slower for the LCP
  solve/symbolic rebuild loop).
- Workspace layout at a glance (one paragraph, link to the dev guide's crate tour for detail):
  `crates/lcp-solver`, `crates/pwl-devices`, `crates/dae-runtime`, `crates/continuous-blocks`,
  `crates/cscript-ffi`, `crates/general-simulator-cli`.
- Running the test suite (`cargo test --workspace`) as a sanity check after building.

## Source material to adapt from
- `AGENTS.md`'s "Required gates" section for the exact commands.
- `README.md`'s "Development" section.
