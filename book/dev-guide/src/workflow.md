# Required gates and workflow

The three commands every piece of work runs before it may be called done, verbatim from
`AGENTS.md`'s "Required gates" section:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
```

They are the top of `AGENTS.md`'s authority order ("1. Mechanical gates: formatters, Clippy,
tests.") and they are *gates*, not suggestions: Clippy runs with `-D warnings`, so a warning
fails the build; nothing merges with one. (`README.md`'s "Development" section lists the same
three commands in a different order — the set is identical.) There is deliberately **no
pre-commit hook in this repo yet**: `CLAUDE.md` says so outright, and points at the
`general-mna` sibling as the repo that has one — until that's set up here, the gates are run
manually, and the journal entries record the result of running them at the bottom of every
non-trivial change (grep `docs/journal/2026-08.md` for `cargo fmt --all` and you'll find one
per entry).

## Commit conventions

From `AGENTS.md`'s "Commit rules":

- Conventional Commits: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`,
  `ci`, `chore`, or `revert` — matching the sibling repos' own convention.
- Never commit unless explicitly asked. Never use `--no-verify` or similar bypasses.

The commit history itself demonstrates the convention (`docs:`-prefixed documentation
commits, `feat:`-prefixed feature commits) — read it the way you'd read the journal: newest
first.

## "No Newton-Raphson, no voltage limiting, anywhere" is a boundary, not a preference

`AGENTS.md`'s "Project boundaries" states it as a rule with teeth:

> **No Newton-Raphson, no voltage limiting, anywhere in this codebase.** That is the entire
> point of the project. If a design under consideration needs continuous per-iteration
> nonlinear solving of device physics, it has left this project's scope — stop and reconsider,
> don't implement it "temporarily."

The rest of that section fills in what the boundary implies, in both directions:

- **Don't re-implement the siblings.** Netlist parsing stays in `general-spice-core` (the
  `spice-lsp` repo); linear-device MNA stamping and Schur-complement reduction stay in
  `general-mna`. This repo's job is exactly two things: LCP-based mode selection for
  piecewise-linear devices, and compiling continuous blocks into descriptor-DAE fragments of
  the shape $A x + K\,\frac{dx}{dt} = B u$. Duplicating work the siblings already do correctly
  is a boundary violation, the same way introducing Newton iteration would be. The full
  sibling-repo rules are `project-boundaries.md`'s subject.
- **Build the risky piece first, in isolation.** `crates/lcp-solver` is "the highest-risk,
  least-familiar numerical piece," and the rule is to build and verify it in complete isolation
  from circuit code — no `pwl-devices` or `dae-runtime` dependency beyond calling its public
  API — trusting it on its own hand-solved fixtures before anything builds on top. That one
  sentence is the seed of the whole incremental-verification order below.
- **The numeric stack is Rust throughout, with one deliberate exception.** No C/C++ FFI
  except the explicit, opt-in `kind=cscript` escape hatch (`crate-cscript-ffi.md`); `faer` is
  deferred until circuit size/sparsity justify it (`crates/dae-runtime/src/linsolve.rs`'s module
  doc comment commits to it "once circuit size or sparsity actually make it worth the
  dependency — revisit then, not before"). Default integrator: implicit trapezoidal, with
  backward Euler as the startup step and as the fallback immediately after a mode switch —
  `dae-integration.md` and `ringing-and-fallback.md` carry the derivations.

The practical instruction for a design review: if the mechanism under discussion needs
continuous per-iteration nonlinear solving of device physics, it has left the project's scope.
The answer is to stop and reconsider — the project's entire premise (every active device
piecewise-linear, so a fixed segment combination is exactly linear, and mode selection is a
discrete LCP solve) is `architecture-overview.md`'s subject, and a design that can't be
expressed that way is a different project. There is no "temporary" Newton-Raphson: a
temporarily-introduced voltage limiter is still a voltage limiter, and it silently changes
what every downstream comparison against this project's own verification baselines means.

## Verification discipline, in one sentence

`AGENTS.md`: "Numerical agreement alone is not proof." For any new device model, LCP
formulation, or continuous block, the expected result must be derived independently — by hand,
or from an independently-worked KCL/KVL/transfer-function derivation — and *that* gets tested,
never just internal self-consistency. End-to-end transient results are compared against the
previously validated Xyce runs in an internal validation-experiment archive's
converter-benchmark and dual-active-bridge (DAB) experiment folders wherever a matching
topology exists — real baselines, not something to re-invent. The full argument (including the
concrete incident that proves a wrong model can produce an unsuspicious-looking number) is
`verification-discipline.md`; do not weaken it to "the tests pass."

The same discipline applies to documentation, and it is mechanized: every `kind=` component's
reference entry must have its Examples and Errors claims verified against a real run of the
CLI before the doc comment is written — `doc-verify/<kind>/test_<kind>.py`, committed alongside
the code it verifies (see `.claude/skills/write-component-doc/SKILL.md`). A doc comment whose
claims were never run is not a finished entry.

## The incremental-verification build order is a continuing pattern

The milestone order recorded in `README.md`'s "Status" section — `lcp-solver` trusted on its
own fixtures first, then `pwl-devices` against it, then `dae-runtime` folding both into one
LCP, then `continuous-blocks` standalone, then `general-simulator-cli` on top — was not a
historical accident. It is the pattern to keep using for any new crate or major feature:
**build and trust one layer in complete isolation before the next layer is allowed to depend
on it.** `crate-tour.md` draws the dependency diagram this produced. A new numerical layer
(e.g. the `faer` migration, or a new device-model family) should get its own standalone
fixtures with independently hand-derived expected values *before* anything wires it into a
circuit's global system — the same sentence `crates/continuous-blocks/src/lib.rs` uses to
justify its own standalone crate boundary ("every block here is verified against a
hand-derived result on its own before anything wires it into a whole circuit's global
system").

## The two continuity mechanisms

Read the newest entry in `docs/journal/` when continuing prior work; record a dated entry for
every non-trivial change, timestamped with `date '+%Y-%m-%d %H:%M'` (never a guessed or
ambient date); record a reproducible trap in `docs/gotchas/` rather than letting the next
contributor rediscover it. Both conventions are spelled out in `journal-and-gotchas.md`.

## Source material this was adapted from

- `AGENTS.md` — "Required gates", "Commit rules", "Project boundaries" (the no-Newton-Raphson
  rule and the lcp-solver-isolation rule), "Numeric stack", "Verification discipline",
  "Journal and gotchas", "Authority order".
- `CLAUDE.md` — the no-pre-commit-hook-yet note.
- `README.md` — "Development" (same three commands) and "Status" (the milestone-by-milestone
  incremental verification order).
- `crates/continuous-blocks/src/lib.rs` — the standalone-crate verification rationale quoted
  above.
- `crates/dae-runtime/src/linsolve.rs` — the `faer` deferral commitment.
- `book/dev-guide/src/verification-discipline.md` — linked, not duplicated.
