# Introduction

This guide is for contributors, reviewers, and anyone who wants to trust the simulator enough
to rely on its results for a real design decision. If you're writing netlists and want to know
what `kind=` a block needs or what a gate binding does, that's the [User
Guide](../user-guide/introduction.md) instead — this guide doesn't repeat that material.

For every major design choice in the codebase, this guide tries to give three things, not just
one: the math, the internal mechanism that implements it, and the rationale — why this
approach and not one of the obvious alternatives. A chapter that only explains *how* the code
works without also explaining *why it's built this way* is treated as incomplete.

## How the guide is organized

- **Architecture** — why this project resolves piecewise-linear device behavior as a Linear
  Complementarity Problem instead of Newton-Raphson, and the descriptor-DAE shape that unifies
  circuits and continuous blocks into one system.
- **Switching** — the diode/ideal-switch models in detail, and the endogenous-vs-exogenous
  distinction that decides which switching decisions the LCP resolves and which ones the caller
  must supply.
- **The block graph** — how continuous and discrete blocks (transfer function, state-space,
  PID, and the rest) get folded into that same descriptor system, evaluated in causal order,
  and checked for algebraic loops.
- **Crate tour** — a paragraph per workspace crate, and the build-up-trust order each one was
  verified in before anything was allowed to depend on it.
- **Contributing** — the verification discipline this project holds itself to, the required
  gates, and the journal/gotchas conventions that keep institutional knowledge from evaporating
  between contributors.
- **Design rationale and open questions** — the decisions log, and the extension points that
  are genuinely still open rather than just undocumented.

## What background this assumes

Comfort with linear algebra (matrix/vector notation, linear systems) and a basic familiarity
with ODEs/DAEs (what a derivative-in-time system is, roughly what "implicit" vs "explicit"
integration means) is assumed throughout. Nothing here assumes prior SPICE-internals knowledge
— the Architecture section builds up the LCP formulation from scratch precisely because it's
not the standard approach.

## When this guide, the code, and the journal disagree

They will, eventually — code moves faster than prose. This project's own authority order
(`AGENTS.md`) is: mechanical gates (formatters, Clippy, tests) first, then any relevant skill
under `.claude/skills/`, then `AGENTS.md` itself, then `docs/architecture.md` and the rest of
`docs/`. This guide sits alongside that last tier: it's written to be verified against the
current source, not against how the source used to look, so if a chapter and the code it
describes genuinely disagree, treat the code as correct and the chapter as stale — and say so
in a journal entry or PR, since that's exactly the kind of drift the project's own verification
discipline exists to catch. `docs/journal/` is the tiebreaker for *why* something is the way it
is, when neither the guide nor a doc comment says — it's a dated, append-only record of actual
decisions made, not a second copy of the reference material.
