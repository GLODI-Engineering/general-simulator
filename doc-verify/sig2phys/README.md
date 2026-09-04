# doc-verify/sig2phys

Verification fixtures for the `Sig2Phys` component reference entry
(`general-mna/src/block_graph.rs`, `BlockKind::Sig2Phys`). Run `test_sig2phys.py` before
editing that doc comment. Replaces the former `doc-verify/sig2voltage`/`doc-verify/sig2current`
folders — `Sig2Voltage`/`Sig2Current` were merged into one `Sig2Phys` variant parameterized by
`domain=voltage`/`domain=current`.

## Files

- `example_voltage.cir` — the doc comment's own `domain=voltage` `## Example`: a block-driven
  voltage source, `V(a)` tracking `CMD` exactly.
- `example_current.cir` — the doc comment's own `domain=current` `## Example`: a block-driven
  current source, `V(a)` matching Ohm's law over `R1=1k` exactly.
- `error_missing_domain.cir` — `domain=` omitted entirely, rejected at parse time.
- `error_invalid_domain.cir` — `domain=power` (neither `voltage` nor `current`), rejected at
  parse time.
- `error_voltage_source_names_current_domain.cir` — a `V` source's own literal value naming a
  `domain=current` converter — rejected (`DaeError::SourceNotSig2PhysicalConverter`); a `V`
  source needs `domain=voltage` specifically.
- `error_converter_wired_as_a_node.cir` — the converter's name used as an ordinary circuit node
  on an element line instead of being referenced by name — rejected
  (`DaeError::Sig2PhysUsedAsCircuitNode`) in `--mode dc` and `--mode transient` alike. This one
  used to *succeed*, silently reporting `V(VDRV) = 0`; the `## Errors` entry for it is still
  owed in `general-mna`'s own `BlockKind::Sig2Phys` doc comment (that repo was not touched by
  the fix, which lives entirely in `dae-runtime`/`general-simulator-cli`).

## Running

```bash
cargo build --release -p general-simulator-cli   # from the repo root, once
python3 doc-verify/sig2phys/test_sig2phys.py
```
