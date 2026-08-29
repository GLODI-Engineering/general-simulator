# `kind=measure`: post-processing measurements, not a block-graph kind

*(Implemented this session — this chapter records the architecture decision and the actual
integration, mirroring `python-blocks.md`'s own before/after structure.)*

## What this is, and what it isn't

`kind=measure` is a netlist line that computes a single ngspice-`.measure`/Xyce-`.MEASURE`-style
scalar (or, for `four`, a small handful of related scalars) from an already-**completed**
transient trace. See `book/user-guide/src/measurements.md` for the full user-facing field
reference; this chapter is the "why here, why like this" the user guide deliberately points back
to rather than re-deriving.

It is **not** a `general-mna::BlockKind` variant, and `dae-runtime`'s block graph never
evaluates it. A measurement has nothing to do with circuit topology or per-step block
evaluation — it's a single reduction over the *whole* resolved waveform, run exactly once, after
the transient loop has already finished. Putting it in the block graph would mean:

- `general-mna` would have to depend on a report-generation crate
  (`gs-waveform-measurements`) — architecturally backwards for a project whose own
  `AGENTS.md` scopes `general-mna` specifically to "symbolic MNA + block graph," not report
  generation (see `book/dev-guide/src/project-boundaries.md`'s sibling-repo map).
- Every measure type would need a per-step evaluation story even though none of them are
  meaningfully defined mid-trace — `MAX`/`RMS`/`INTEG`/`FOUR`/`TRIG`-`TARG` are all definitions
  *over an interval*, not a per-instant value a block graph could sensibly output at each step.
- `dae-runtime`'s adaptive step-size controller would gain a spurious dependency on something
  that has no bearing on local truncation error at all.

So `kind=measure` is recognized, parsed, and evaluated entirely inside
`general-simulator-cli` (`crates/general-simulator-cli/src/measure.rs`), as a post-processing
pass sitting between "the transient trace is fully resolved" and "print the results" — the same
place `--format csv`/`--format raw` already sit. `general-mna` and `spice-lsp` were **not**
changed at all for this feature (see "Why no sibling-repo changes were needed" below).

## Where the actual math comes from

None of the measurement math is reimplemented here — `measure.rs` is a thin field-parsing/
dispatch layer over [`gs-waveform-measurements`](../../../gs-waveform-measurements), a
standalone sibling crate with no dependency on any circuit or block-diagram type (see its own
`src/lib.rs` doc comment). Read that crate's docs directly for the numerical techniques ($\Delta
t$-weighted trapezoidal/exact-quadrature scalar reductions, straddling-sample linear
interpolation for every crossing, exact per-segment analytic Fourier integration for `FOUR`) and
why each one matters specifically for a variable-timestep solver's own non-uniformly-spaced
output — this chapter doesn't re-derive any of that.

Two real ngspice/Xyce measure types are not available here, because
`gs-waveform-measurements` itself deliberately doesn't implement them (not merely deferred by
this integration layer):

- **`EQN`** — evaluating an expression over *other measurements'* own results. That's an
  orchestration-layer concern (this integration layer, in principle), not a per-signal waveform
  primitive the measurement crate should own. Nothing here builds it either; a netlist author
  wanting this today has to compute it themselves from the printed `name = value` lines.
- **The `FILE=` half of `ERROR`** — parsing a reference-waveform file format. The norm
  computation itself (`type=error`, comparing two given time series) *is* implemented; only
  reading a *third* file's own waveform format is out of scope. `ref=` in this netlist grammar
  must name a signal already present in the same run's own trace.

## How `kind=measure` lines are kept away from `general-mna`

`general_mna::build_system`'s own `kind=` dispatch (`system_builder.rs`'s `build_kind`) has no
entry for `"measure"`, and its final fallback arm — after trying the diode/ideal switch/every declared
block kind, then the waveform-arithmetic/logic-gate/flip-flop fallback ladder — is
`other => Err(format!("line {}: unknown device kind '{other}'", ...))`. So a `kind=measure` line
handed to `build_system` unmodified would simply fail to parse, the same as any genuinely
unrecognized `kind=` value.

`measure::extract` (`crates/general-simulator-cli/src/measure.rs`) runs *before* that call:

1. It calls `general_mna::parse_and_flatten` (already `pub`) on the netlist text itself, to get
   the real `Vec<general_spice_core::ast::Statement>` — the exact same parse `build_system`
   would otherwise do internally.
2. It picks out every `Statement::BlockInstance` whose own `kind` field is `"measure"`, parses
   its fields into a `MeasureSpec`, and records the 1-based source line range
   (`BlockInstance::span`) each one came from.
3. It rewrites the netlist text with **exactly those line ranges blanked out** (replaced with an
   empty line, not deleted) and returns that alongside the collected specs. Blanking rather than
   deleting keeps every *other* statement's own line number identical to the original file, so a
   `general-mna` parse error on some other line still points at the line a user sees in their
   own editor.
4. `main.rs` passes the *blanked* text to `general_mna::build_system` — which therefore never
   sees `"measure"` as a `kind=` value at all, and needs no changes of its own to tolerate it.

This is a textual, line-level filter, not a semantic one — it works because `BlockInstance`
statements are single-purpose lines with their own span, and because `general_spice_core`'s
lexer already treats a blank line exactly like a blank line (no special "this used to be
something" bookkeeping needed on either side).

## Why no sibling-repo changes were needed

The task this feature was scoped from anticipated possibly needing a small, explicitly-scoped
change to `general-mna` (e.g. "tolerate an unrecognized `kind=` it's told about in advance") or
to `spice-lsp`. Neither was necessary: `general_mna::parse_and_flatten` was already `pub`, and a
textual line-blanking pass entirely on the CLI side was sufficient to keep `general-mna` from
ever seeing the new `kind=` value. Both sibling repos are unmodified by this feature — no new
journal entries were added there.

## Signal naming: reusing the existing trace column convention

A `kind=measure` line's `out=`/`ref=`/`when=`/etc. fields name a signal by exactly the column
name `--format csv`/`--format raw` already print for it — `V(<node>)` for a node voltage, or a
declared block's own name (see `book/user-guide/src/reading-output.md`) for a block output.
There is deliberately no new naming syntax: `measure::samples_of` looks the name up directly in
the already-built `Waveform`'s own `headers`/`rows`, the same data `print_csv`/`raw_format`
already consume. This keeps `kind=measure` from needing its own notion of "what a signal is" —
it's exactly whatever the rest of this CLI already knows how to print.

## Output: why stderr, and why that doesn't corrupt the existing formats

Measurement results print to **stderr**, one `name = value` line per result (ngspice's own
`.measure` printed convention), regardless of `--format`. This was a deliberate, explicit
choice among three options considered:

1. **Stdout, after the CSV.** Rejected: a tool reading the CLI's stdout as a full CSV stream
   (e.g. `pandas.read_csv(sys.stdin)`, or any of this project's own existing CLI tests) would
   see trailing non-CSV lines and either error or silently misparse. `--format csv`'s own
   documented contract (unchanged since before this feature existed) is "prints
   `t,V(node1),...` to stdout, one row per point" — full stop; appending anything else would be
   a silent, undocumented change to that contract for the (common) case of a netlist that
   happens to also use `kind=measure`.
2. **A new `--measure-out <path>` flag.** Rejected as unnecessary complexity for the first cut of
   this feature: nothing else in this CLI writes a *third* kind of output file, and stderr is
   already unambiguously "not the machine-readable trace" without inventing new plumbing. This
   remains the natural place to add such a flag later if a real need for a machine-readable
   measurement file (JSON, a second CSV, ...) comes up.
3. **Stderr (chosen).** Keeps stdout's own contract — a plain CSV under `--format csv`, or
   nothing at all under `--format raw` (which already writes to a file, not stdout) —
   byte-for-byte unaffected by whether the netlist has any `kind=measure` lines, while still
   printing every result somewhere a human (or a script specifically parsing stderr, as
   `tests/measure/_lib.py` does) can see it immediately, without a new flag.

A failed individual measurement (an unknown signal name, a crossing that never occurs in its own
window) is printed the same way, on its own line, and does not abort the run or any other
measurement — `measure::evaluate_all` returns one independent `Result` per spec, exactly so one
bad measurement can't hide the others' results.

## Testing

Every measure type `gs-waveform-measurements` implements is exercised end-to-end through the
real CLI binary in `tests/measure/` (see that directory's own `README.md` for the full
type-to-fixture mapping) — not a couple of representative cases. Every fixture drives its
signal(s) from an exact closed-form source (`kind=pwl` two-point ramps, `kind=sinwave`/
`kind=pulsewave`, i.e. real SPICE `SIN()`/`PULSE()` sources) through an ideal V-source, so every
expected value is a hand-derived closed form, the same discipline
`gs-waveform-measurements`'s own test suite and this project's `doc-verify/` folders already
follow — never "trust the code's own output."

`tests/measure/test_sine_adaptive.py` is the one worth calling out specifically: it runs a pure
sinusoid with **no `--dt` at all** (adaptive stepping), first asserting the resolved trace's own
step size really is non-uniform (a measured >5× ratio between the largest and smallest `dt` in
the trace — a real RC branch elsewhere in the same netlist exists purely to give the adaptive
controller genuine dynamics to react to), then asserting `FOUR` still recovers the sinusoid's
known analytic fundamental magnitude/phase and near-zero THD on that same non-uniform trace.
This is the direct, CLI-level proof of the entire reason this architecture (and
`gs-waveform-measurements` itself) exists — a measurement that assumed uniform spacing would get
a subtly wrong answer here, and this test would catch it.
