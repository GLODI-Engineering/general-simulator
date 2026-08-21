# Journal and gotchas conventions

*(Skeleton — outline below; not yet written.)*

## What goes here
- The journal's purpose and discipline: a continuity log, newest entry first, real timestamps
  (`date '+%Y-%m-%d %H:%M'`, never guessed), one entry per non-trivial change, never rewritten
  — a correction is a *new* entry linking the old one, not an edit.
- The gotchas convention: one file per reproducible numerical/pivoting/integration trap,
  symptom/cause/fix/how-to-apply structure, indexed in `docs/gotchas/INDEX.md`.
- Why both matter for *trusting* the simulator specifically (the framing this whole doc effort
  is in service of): a reader can trace not just what the code does today, but what was tried,
  what broke, and why the current design won — the journal is the audit trail verification
  discipline produces.
- A pointer to the `internal-archive` sibling repo's own, separate journal/gotchas
  convention, and why the two are *not* merged (different repos, different scope, cross-linked
  where relevant).

## Source material to adapt from
- `docs/journal/README.md` and `docs/gotchas/README.md` — port directly.
