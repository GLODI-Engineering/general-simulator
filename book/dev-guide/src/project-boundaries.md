# Project boundaries: the sibling repos

This repo sits in a directory with several siblings, and which repo owns which piece of the
system is a *rule*, not a convenience. `README.md`'s "Relationship to the sibling repos"
draws the map; `AGENTS.md`'s "Project boundaries" section gives it teeth; and the journal
records three concrete incidents where the rule was exercised. The path dependencies that
make the map load-bearing are in `crates/dae-runtime/Cargo.toml` and
`crates/general-simulator-cli/Cargo.toml`:

```toml
general-spice-core = { path = "../../../spice-lsp/servers/core", version = "0.1.0" }
general-mna = { path = "../../../general-mna", version = "0.1.0" }
gs-waveform-measurements = { path = "../../../gs-waveform-measurements", version = "0.1.0" }
```

## The map

- **`spice-lsp`** (containing the `general-spice-core` crate) — the **sole netlist-parsing
  authority**. The lexer/parser, the AST (`Statement`, `ElementInstance`,
  `BlockInstance`), the dialect handling, `.subckt`/`X`-instance flattening. This repo
  consumes parsed statements; it never re-parses raw text on its own (the one exception-like
  detail — `dae-runtime`'s `topology.rs` walking the *caller's own* statement list for
  diode node names — operates on the already-parsed AST for exactly that reason).
- **`general-mna`** — the **sole implementation of linear-device MNA stamping** (R, C, L,
  V, I, G, E, F, H) and numeric Schur-complement reduction, plus `TransientFunction`
  (`SIN`/`PULSE`/`EXP`/`PWL`/`SFFM`) source support, and — since the format-unification
  work — the netlist-facing block vocabulary itself: `build_system` parses every `kind=`
  line into `BlockKind`/`BlockInstance` values, and `dae-runtime`'s block graph re-exports
  those types rather than defining its own. `README.md`: "this repo reuses those directly
  rather than re-deriving them."
- **`general-simulator`** (this repo) — adds exactly two things, per `README.md` and
  `AGENTS.md`: (1) LCP-based mode selection for piecewise-linear devices, and (2) compiling
  continuous blocks into descriptor-DAE fragments. Everything else is someone else's job,
  and doing it here is a boundary violation — `AGENTS.md`: "Do not duplicate work the
  sibling repos already do correctly."
- **`gs-waveform-measurements`** — extracted *out* of this workspace to its own sibling repo
  (2026-08-29, per `Cargo.toml`'s workspace-member comment and the journal entry of that
  date) "for an independent release cadence and reuse beyond this workspace"; consumed by
  `general-simulator-cli` alone, to evaluate `kind=measure` lines as post-processing over a
  completed trace.
- **an internal validation-experiment archive** — the *other* direction, see below.

## The escalation rule

`AGENTS.md`, verbatim: the sibling repos are read-only path dependencies — "Do not edit
those repositories unless the user explicitly expands the task to them. If a change there
turns out to be genuinely necessary, propose it explicitly and get confirmation before
editing that repo." Three journal case studies show what following the rule actually looks
like:

1. **The flagged-early case** (2026-08-17, Milestone 1 entry): before `dae-runtime` even
   existed, the Milestone-3 design needed a decision about how `dae-runtime` would get
   linear stamping — extend `general-mna`'s public API or stamp locally. The entry ends:
   "Flagged for the user before touching the `elspice-mna` repo, since any change there is
   a cross-repo decision outside this repo's own boundary."
2. **The verified-non-change** (2026-08-22 22:58): the signal-write-direction converter
   looked like it needed a `general-mna` change; the entry records "**Confirmed before
   writing anything cross-repo**" — `Expression::parse_scalar` already accepted a bare
   symbol and `MnaSystem::evaluate` already substituted caller-supplied values, both
   "confirmed by direct reading of `elspice-mna/src/{expression,numeric,system}.rs`, not
   assumed" — so the feature needed **zero** changes to the sibling repo. The rule's
   payoff in one sentence: checking first turned a cross-repo change into none.
3. **The two-step rename** (2026-08-29 17:57 and 18:24) — the case study the skeleton this
   chapter replaced asked for. `pwl-devices` renamed `Diode`/`Mosfet` to
   `IdealDiode`/`IdealSwitch`, but `general-mna` (read-only) still imported the old names
   and owned the `kind=mosfet`/`kind=diode` netlist keywords. The resolution was explicitly
   two-step: keep compatibility type aliases so `general-mna` compiled unchanged;
   **intentionally leave `book/user-guide/src/component-reference.md` stale**, because it is
   generated from `general-mna`'s own `GateBinding::Block` doc comment and hand-editing it
   "would just be silently lost and would drift from its own source of truth" — it would
   keep saying MOSFET "until `general-mna`'s own doc comment is updated in a separate,
   explicitly-scoped change." The follow-up entry (18:24) records exactly that happening:
   `general-mna` was itself updated in the same direction, the aliases were dropped, and
   the reference was regenerated. The docs being *briefly stale* was the correct, recorded
   behavior — not an oversight — because the alternative (editing generated output or
   editing the sibling repo uninvited) is worse.

## The internal validation-experiment archive direction

The same boundary, pointed the other way: this repo's own experiments/validation baselines
live *there*, not here. `README.md`'s "Status" section: validation against the Xyce/ngspice
baselines "already captured in the sibling archive's `experiments/`
folder." `AGENTS.md`'s "Verification discipline": end-to-end transient results are compared
"against the existing, previously validated Xyce runs already captured in that
sibling archive's `experiments/converters-benchmark-*` and
`experiments/dab-*` folders wherever a matching topology exists — those are real baselines,
not something to re-invent." The division of labor is the same one from
`journal-and-gotchas.md`: **this repo is the simulator; that one is where it gets
exercised, benchmarked, and written up as worked examples.** A converter experiment's
netlists, plots, and write-ups belong there; the simulator feature they motivated (the
dynamic-clamp PID, the signal converters) belongs here, with a journal entry linking across.

## Source material this was adapted from

- `README.md` — "Relationship to the sibling repos", "Status".
- `AGENTS.md` — "Project boundaries", "Verification discipline".
- `crates/dae-runtime/Cargo.toml`, `crates/general-simulator-cli/Cargo.toml` — the sibling
  path dependencies quoted above.
- `Cargo.toml` — the `gs-waveform-measurements` extraction comment.
- `docs/journal/2026-08.md` — 2026-08-17, 2026-08-22 22:58, 2026-08-29 17:57 and 18:24
  entries.
