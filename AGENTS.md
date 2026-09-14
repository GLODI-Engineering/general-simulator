# Agent guidelines

`elspice-pwl` is an educational Rust simulator for piecewise-linear (PWL) circuits and
mixed circuit/block-diagram systems, designed to avoid Newton-Raphson + voltage limiting
entirely by resolving which device segment is active, per timestep, as a Linear
Complementarity Problem (LCP). See [`README.md`](README.md) and
[`docs/architecture.md`](docs/architecture.md) for the full rationale and math.

## Authority order

1. Mechanical gates: formatters, Clippy, tests.
2. Skills in `.claude/skills/<name>/SKILL.md` for recurring workflows.
3. This file.
4. `docs/architecture.md` and other documents under `docs/`.

## Project boundaries

- `spice-core` (from the sibling `spice-lsp` repo) and `elspice-mna` (sibling repo) are
  read-only path dependencies. Do not edit those repositories unless the user explicitly
  expands the task to them. If a change there turns out to be genuinely necessary (see
  `docs/architecture.md`'s note on the `elspice-mna` extension-point question), propose it
  explicitly and get confirmation before editing that repo.
- Keep netlist parsing in `spice-core`. Keep linear-device MNA stamping and Schur-complement
  reduction in `elspice-mna`. This repo's job is exactly two things: (1) LCP-based mode
  selection for piecewise-linear devices, and (2) compiling continuous blocks
  (transfer-function/state-space/PID/...) into descriptor-DAE fragments. Do not duplicate
  work the sibling repos already do correctly.
- **No Newton-Raphson, no voltage limiting, anywhere in this codebase.** That is the entire
  point of the project. If a design under consideration needs continuous per-iteration
  nonlinear solving of device physics, it has left this project's scope — stop and reconsider,
  don't implement it "temporarily."
- Build and verify `crates/lcp-solver` in complete isolation from circuit code (no `pwl-devices`
  or `dae-runtime` dependency on it beyond calling its public API) — it is the highest-risk,
  least-familiar numerical piece and must be trusted on its own hand-solved fixtures before
  anything else builds on top of it.

## Numeric stack

- Rust throughout; no C/C++ FFI. `faer` for sparse/dense linear algebra once the circuit-facing
  crates need it (`lcp-solver` itself uses a plain dense tableau — Lemke's algorithm is
  inherently small and dense, `faer` is not needed there).
- Default integrator: implicit trapezoidal, with backward Euler as the startup step and as the
  fallback immediately after a mode switch (matches SPICE/Xyce's own default and its reasoning
  for switching to backward Euler around a discontinuity).

## Required gates

```bash
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --workspace
```

## Verification discipline

Numerical agreement alone is not proof. For any new device model, LCP formulation, or
continuous block: derive the expected result independently (by hand, or from an
independently-worked KCL/KVL/transfer-function derivation) and test that, not just internal
self-consistency. Compare end-to-end transient results against the existing, previously
validated Xyce runs already captured in an internal validation-experiment archive's
converter-benchmark and dual-active-bridge (DAB) experiment folders wherever a matching
topology exists — those are real baselines, not something to re-invent.

## Documentation

Every netlist `kind=` component gets a reference entry generated from source doc comments —
see `.claude/skills/write-component-doc/SKILL.md` before writing or editing one. **Testing
every documented Example and Errors claim against a real run of the CLI, before writing the
doc comment, is mandatory practice, not optional polish** — the skill's own "Testing Examples
and Errors against a real run" section covers exactly how (`doc-verify/<kind>/test_<kind>.py`,
committed alongside the code it verifies, not scratch). A doc comment whose claims were never
run is not a finished entry.

### Math notation — golden rule, never break it

**Every mathematical expression in any doc comment, book chapter, or markdown file in this repo
is real LaTeX, using mdbook-katex's delimiters (`$...$` inline, `$$...$$` display) — never
plain-text/ASCII notation (`A x + K dx/dt = B u`, `V(b) = 4.26`, `x^2`) written as bare prose,
and never a formula placed inside a ```` ```text ```` code fence as a substitute for real math
markup.** This applies even when the *source material* you're adapting from (a Rust doc
comment, `docs/architecture.md`, a journal entry) itself uses plain-text notation — convert it
to real LaTeX in whatever you write, don't carry the plain-text style forward. Don't use the
`\LaTeX` text-mode macro inside math mode — it breaks the PDF build (`f89cfc3` fixed this exact
bug once already; use plain text like "real math notation" outside `$...$` instead of the
macro). After writing or editing anything with math in it, rebuild the relevant book
(`mdbook build book/user-guide/` / `book/dev-guide/`) to confirm KaTeX actually renders it, and
grep back through what you wrote for any bare expression you missed before calling it done.

## Journal and gotchas

Read the newest entry in `docs/journal/` when continuing prior work. Include a dated entry with
non-trivial changes, timestamped with `date '+%Y-%m-%d %H:%M'` (never a guessed or ambient
date). Record a gotcha (`docs/gotchas/`) for any reproducible numerical, pivoting, or
integration trap another contributor would otherwise have to rediscover — LCP degeneracy and
cycling failure modes are the likeliest candidates in this codebase.

## Commit rules

- Conventional Commits: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`,
  `ci`, `chore`, or `revert` — matching the sibling repos' convention.
- Never commit unless explicitly asked. Never use `--no-verify` or similar bypasses.
