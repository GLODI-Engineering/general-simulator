# Your first netlist

*(Skeleton — outline below; not yet written.)*

## What goes here
- Walk one tiny, complete example end to end: an RC circuit through a diode, or a single
  ideal-switch half-bridge — something already hand-verified in the test suite, not a fresh example.
- Show the *whole* file, annotated: ordinary SPICE element lines, then the `*`-prefixed
  `kind=...` comment lines, explaining the one-file convention (this file is valid SPICE to
  any other tool; `general-simulator-cli` additionally reads the `kind=` comments).
- Run it: `general-simulator <file>.cir --mode transient --tfinal ... --dt ...`, show the CSV output
  shape (`t,V(node1),V(node2),...`).
- Point at `--mode dc` too, briefly (operating-point only, one row of output).

## Source material to adapt from
- `crates/general-simulator-cli/src/main.rs`'s own module doc comment — the whole grammar
  explanation already lives there; this chapter should be the narrated walkthrough version of
  one concrete example from it, not a restatement of the grammar (that's `netlist-grammar.md`).
- `crates/general-simulator-cli/tests/cli.rs` fixtures (`tests/fixtures/two_diode.cir`,
  `rc_diode.cir`) are good, already-verified candidates for the walked example.
