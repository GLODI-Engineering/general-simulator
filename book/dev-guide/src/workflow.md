# Required gates and workflow

*(Skeleton — outline below; not yet written.)*

## What goes here
- The three commands, always, before calling anything done: `cargo fmt --all -- --check`,
  `cargo clippy --all-targets -- -D warnings`, `cargo test --workspace`.
- Commit conventions (Conventional Commits: `feat`/`fix`/`docs`/`style`/`refactor`/`perf`/
  `test`/`build`/`ci`/`chore`/`revert`), matching the sibling repos.
- "No Newton-Raphson, no voltage limiting, anywhere" as a hard project boundary, not a style
  preference — what to do if a design under consideration seems to need it (stop, reconsider,
  don't implement it "temporarily").
- The incremental-verification build order this project actually follows (build/trust one
  crate in isolation before the next depends on it) as a contribution pattern to keep using for
  new crates/major features, not just historical trivia.

## Source material to adapt from
- `AGENTS.md` in full — this chapter is close to a direct port of most of that file, reframed
  for a book reader instead of an agent's own instruction file.
