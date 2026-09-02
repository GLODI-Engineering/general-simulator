# doc-verify/phys2sig

Verification fixtures for the `Phys2Sig` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::Phys2Sig`). Run `test_phys2sig.py` before editing
that doc comment.

## Files

- `example.cir` — the doc comment's own `## Example`: reading a resistor-divider node,
  `PROBE1` tracking `V(out)` exactly.
- `error_both_node_and_branch.cir` — `node=`/`branch=` both given, rejected at parse time.
- `error_neither.cir` — neither given, rejected at parse time.
- `ammeter.cir` — the doc comment's second `## Example`: the classic MNA "ammeter" idiom (a
  0V voltage source inserted in series, then read by name) for measuring the current
  through an element (`R1`) that has no branch-current unknown of its own.
- `error_branch_not_a_branch_device.cir` — `branch=` naming a real element (`R1`, a resistor)
  that isn't V/L/E/H and so has no branch-current unknown — a hard build-time error, not a
  silent `0.0`.
- `error_branch_no_such_element.cir` — `branch=` naming an element that doesn't exist in the
  netlist at all — a distinct build-time error from the wrong-type case above.

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/phys2sig/test_phys2sig.py
```
