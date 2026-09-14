# Workspace layout

The workspace (`Cargo.toml`, root of this repo) has eight members, built in a strict
dependency order that was also the order they were *verified* in — that ordering is not
incidental bookkeeping, it is how the project trusts its own numerics (see
`verification-discipline.md`):

```text
lcp-solver ──► pwl-devices ──► dae-runtime ──► general-simulator-cli
                  ▲                 ▲
                  │                 ├── continuous-blocks
                  │                 ├── cscript-ffi
                  │                 ├── pyblock-ffi   (optional `python` feature)
                  │                 └── octave-ffi    (no build-time deps at all)
```

Three things the diagram deliberately does not draw, because they are path dependencies on
repositories *outside* this one (`crates/dae-runtime/Cargo.toml` pins them with `path =
"../../../..."`; see `project-boundaries.md` for the rules this imposes):

- `general-spice-core` (in the sibling `spice-lsp` repo) — the sole netlist-parsing authority.
- `general-mna` (sibling repo) — the sole implementation of *linear*-device MNA stamping
  (R, C, L, V, I, G, E, F, H) and numeric Schur-complement reduction, plus the
  `TransientFunction` source forms.
- `gs-waveform-measurements` (own sibling repo, extracted 2026-08-29 per `docs/journal/2026-08.md`)
  — consumed by `general-simulator-cli` alone, to evaluate `kind=measure` lines as a
  post-processing pass over a completed transient trace.

What this repo *adds* to those two are exactly two things (`README.md`, "Relationship to the
sibling repos"): PWL device segments resolved via LCP, and a continuous-block library compiled
into descriptor-DAE fragments of the same $A x + K \dot x = B u$ shape `general-mna` already
uses. Anything else — parsing, linear stamping — is deliberately not re-implemented here.

## The build-up-trust order

Each crate was trusted on its own hand-derived fixtures before anything was allowed to depend
on it, and the milestone order in `docs/architecture.md`'s "Status" section records it:

1. **`lcp-solver`** — Lemke's algorithm on a dense simplex tableau, verified against
   hand-solved textbook LCP fixtures, with zero knowledge of circuits or `general-mna`
   (its `src/lib.rs` says so outright). Built in complete isolation: `AGENTS.md`'s project
   boundaries call it "the highest-risk, least-familiar numerical piece."
2. **`pwl-devices`** — the 3-segment `IdealDiode` and the `IdealSwitch`, verified against
   hand-derived circuit operating points. Named *ideal switch* rather than *MOSFET* because
   that name is reserved for a future, not-yet-implemented BSIM-style model
   (`docs/journal/2026-08.md`, the 2026-08-29 rename entries).
3. **`dae-runtime`** — folds a netlist's linear part plus PWL devices into one LCP, with the
   full transient loop (trapezoidal, backward Euler on the first step and any LCP-resolved
   mode change), ideal-switch/PWM support, and the block-graph evaluation machinery.
4. **`continuous-blocks`** — the block library, deliberately standalone (no dependency on the
   three crates above it), verified block-by-block against hand-derived results.
5. **`general-simulator-cli`** — the `general-simulator` binary: netlist in, CSV (or
   `--format raw`) waveform out, plus `kind=measure` post-processing.

The escape-hatch crates (`cscript-ffi`, `pyblock-ffi`, `octave-ffi`) sit beside `dae-runtime`
rather than above it: they define call contracts for user-supplied code that
`dae-runtime`'s block graph invokes, and their verification is of the contracts themselves
(each has its own test suite driving real compiled/interpreted fixtures).

## One paragraph per crate

- [`lcp-solver`](crate-lcp-solver.md) — Lemke's algorithm for `w = Mz + q, w, z >= 0, w.z = 0`.
  Pure numerics, no circuit knowledge, testable in isolation.
- [`pwl-devices`](crate-pwl-devices.md) — PWL device *curves*: the 3-segment ideal diode, its
  Chua-Lin canonical decomposition (the `max(0, ...)` terms that become LCP `z` variables), and
  the ideal switch (gated channel + body diode). Curves only; folding devices into a circuit's
  `(M, q)` is `dae-runtime`'s job.
- [`dae-runtime`](crate-dae-runtime.md) — circuit assembly and the transient loop: the generic
  Thevenin/LCP fold, diode-only and block-graph transient drivers, adaptive step control, and
  the block-graph evaluator (`BlockState`, `evaluate_blocks`, `topological_order`).
- [`continuous-blocks`](crate-continuous-blocks.md) — the block library: state-space /
  transfer-function / PID / discrete-PID compilation into descriptor-DAE fragments, the
  bespoke-`step()` blocks (VCO, hysteresis, PMSM), coordinate transforms, logic, and the
  stateless math ops.
- [`cscript-ffi`](crate-cscript-ffi.md) — the native escape hatch: loads a user-supplied
  precompiled shared library and calls it once per step. The one place arbitrary native code is
  deliberately allowed.
- `pyblock-ffi` — the Python-hosted counterpart, embedding an interpreter via PyO3 and calling
  user `.py` functions once per step. Gated behind the optional `python` feature (a
  build-time `pyo3`+`libpython` dependency). See `python-blocks.md` and `pyfunc-blocks.md`.
- `octave-ffi` — the Octave-hosted counterpart, driving one persistent `octave-cli` *subprocess*
  (never a linked library — GPLv3; "mere aggregation" instead). No build-time dependency at
  all. See `octave-blocks.md`.
- [`general-simulator-cli`](crate-general-simulator-cli.md) — the `general-simulator` binary:
  argument parsing, `kind=measure` stripping, CSV/rawfile writing, and the tests that spawn the
  real binary against already-hand-verified fixtures.

## Source material this was adapted from

- `README.md` — "Relationship to the sibling repos" and "Status" (the milestone list this
  chapter's build-up-trust order follows).
- `docs/architecture.md` — the milestone framing and the two-things-this-repo-adds boundary.
- `Cargo.toml` — workspace members and the `gs-waveform-measurements` extraction comment.
- `crates/dae-runtime/Cargo.toml`, `crates/general-simulator-cli/Cargo.toml` — the real
  cross-repo path-dependency edges drawn in the diagram.
- `AGENTS.md` — "Project boundaries" (the lcp-solver-isolation rule quoted in step 1).
- Each crate's own `src/lib.rs` (or `src/main.rs`) module doc comment for its one-paragraph
  summary.
