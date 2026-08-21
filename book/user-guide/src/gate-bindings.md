# Gate bindings (fixed, PWM, block-driven)

*(Skeleton — outline below; not yet written.)*

## What goes here
- Full field reference for each `gate=` variant: `on`/`off`, `pwm`, `dutyctrl`,
  `dutyctrlcomplement`, `vco`, `dutyctrl`+block, `block`, `vcophase`.
- A table: which variant needs a block graph at all, which is purely numeric/fixed.
- The half-bridge pattern specifically: `dutyctrl`/`dutyctrlcomplement` sharing one duty block
  and `freq=`, and *why* that guarantees no shoot-through/no gap (same carrier, exact logical
  complement) — worth a small timing diagram.
- Cross-reference: this is "how a gate's *state* is decided"; `signals.md` covers how a block's
  own *inputs* are wired, a related but different question.

## Source material to adapt from
- `crates/dae-runtime/src/block_graph.rs`'s `GateBinding` enum doc comments — each variant is
  already documented at exactly this level of detail.
- `crates/elspice-pwl-cli/src/main.rs`'s module doc comment, the `gate=` bullet list.
