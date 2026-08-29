# doc-verify/gatebinding

Verification fixtures for the `Gate Binding` component reference entry
(`general-mna/src/block_graph.rs`, `GateBinding::Block`). Run `test_gatebinding.py` before
editing that doc comment.

## Files

- `example.cir` — the doc comment's own `## Example` (`general-mna`'s `GateBinding::Block`,
  which still says "MOSFET" in its own prose -- `general-mna` is a read-only sibling repo, out
  of scope for `general-simulator`'s ideal-switch rename): a `kind=mosfet` ideal switch held
  permanently on via `gate=block ctrl=<sig2voltage>`. Checked that `V(out)` settles to
  `5 * 1000 / (1000 + 0.1)` (a fully-on 0.1 Ω ideal switch in series with a 1 kΩ load).

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/gatebinding/test_gatebinding.py
```
