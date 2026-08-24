# Project boundaries: the sibling repos

*(Skeleton — outline below; not yet written.)*

## What goes here
- The sibling-repo map: `spice-lsp`/`general-spice-core` (sole netlist-parsing authority),
  `general-mna` (sole linear-device MNA stamping + numeric Schur-complement reduction, plus
  `TransientFunction` source support), both read-only path dependencies from this repo's point
  of view.
- The explicit escalation rule: a change to a sibling repo needs the user's confirmation before
  it happens, even when a change there turns out to be genuinely necessary — cite the real
  precedent (`general-mna` gained `TransientFunction`/`SIN`/`PWL` support in exactly this way,
  and this crate's own docs were briefly stale about that until corrected).
- `internal-archive`, the *other* direction: this repo's own experiments/validation
  baselines live there, not here — what belongs in each repo and why (this repo is the
  simulator; that one is where it gets exercised, benchmarked, and written up as worked
  examples).

## Source material to adapt from
- `AGENTS.md`'s "Project boundaries" section.
- `general-simulator`'s journal entry documenting the stale-docs correction (SIN/PWL support) as the
  concrete case study for "why this rule exists and what happens when it's followed."
