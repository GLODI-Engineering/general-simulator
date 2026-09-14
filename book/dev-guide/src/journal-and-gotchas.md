# Journal and gotchas conventions

Two continuity mechanisms, both in `docs/`, both named in `README.md`'s "Development"
section as the things to consult before touching anything: "Continuity log:
[`docs/journal/`](docs/journal/). Read the newest entry before continuing prior work.
Non-obvious traps: [`docs/gotchas/INDEX.md`](docs/gotchas/INDEX.md)." They exist for the
same reason the verification discipline does — a reader should be able to trace not just
what the code does today, but what was tried, what broke, and why the current design won.

## The journal

`docs/journal/README.md` states the rules, ported here with the observed practice filled in:

- **Chronological, newest first.** Entries live in one file per month, `YYYY-MM.md`
  (`2026-08.md`, `2026-09.md`), the newest entry at the top of the current month's file.
- **Real timestamps, never guessed.** Every entry opens with a
  `## YYYY-MM-DD HH:MM — title` heading; `AGENTS.md` adds the rule verbatim: obtain the
  timestamp with `date '+%Y-%m-%d %H:%M'`, "do not trust an ambient date."
- **One entry per non-trivial change.** The README: "Each entry records what was asked, what
  was found, what changed, verification, and what remains." The real entries follow that
  shape in practice — e.g. the 2026-09-11 entry on the `ic=` solve→assignment change walks
  "What changed upstream / What changed here / New test / Documentation / Gate", and the
  2026-08-29 17:57 rename entry has an explicit "What did NOT change, and why" section that
  is half the entry's value (it records which sibling repo blocked the full rename and why —
  see `project-boundaries.md`).
- **Never rewritten.** The README: "Do not rewrite history. If work is reverted or corrected,
  add a new entry that links to the earlier one." Past entries keep their historical naming
  (the journal still says `elspice-pwl`/`elspice-mna`/`kind=mosfet` where the current code
  says `general-simulator`/`general-mna`/`kind=ideal_switch`); the rename entries are the
  record of the change, not an excuse to edit what came before.
- **The journal records designs; the other two places record their own kind of knowledge.**
  README again: "Use `docs/architecture.md` for lasting design rationale and `docs/gotchas/`
  for a reproducible trap." A journal entry can link to either, but shouldn't be the only
  home for a design decision or a trap.

## The gotchas

One file per reproducible numerical/pivoting/integration trap, so the next contributor
doesn't rediscover it — `AGENTS.md` names LCP degeneracy and cycling failure modes as the
likeliest candidates. The conventions, as observed in `docs/gotchas/`:

- `_TEMPLATE.md` defines the structure: YAML-style front matter
  (`id: GOTCHA-NNN`, `discovered`, `discovered_by`, `scope`, `severity`, `status`,
  `reproducibility`, `tags`), then the sections **Symptom** (quote the exact failure or
  describe the silent divergence), **Reproduction** (minimum circuit/netlist/environment/
  command and expected-vs-actual), **Root cause** (state `unknown` plus the strongest leads
  if that's what it is), **Fix or workaround** (smallest reliable mitigation and its
  trade-offs), **Prevention** (the test/lint/hook that would catch it early), **References**,
  **History** (dated one-liners).
- `INDEX.md` is a manually maintained table (`ID | Title | Severity | Status | Scope |
  Discovered`), with a note that there is deliberately no automation script for it yet —
  "unlike `general-mna`."
- The one real entry so far, `GOTCHA-001-cscript-ffi-fixture-race.md`, is the worked example
  of the format: symptom quoted verbatim (`Load { path: "...", message: "dlopen failed" }`
  panics, a *different* subset of tests each run), root cause a parallel-test race on one
  shared temp `.so` path (one thread's `cc -shared` truncating the file while another
  thread's `dlopen` reads it), workaround `cargo test -p cscript-ffi -- --test-threads=1`,
  status `open`, severity `low`, reproducibility `intermittent`.

## Why these matter for *trusting* the simulator

The verification discipline (`verification-discipline.md`) produces two artifacts: tests
with hand-derived expected values, and the journal entries that say how each milestone was
derived, what broke along the way (the Lemke pivot-rule bug, the `expwave` `td2` default
bug, the reversed body-diode footgun — each a dated entry), and what was deliberately left
open. The gotchas index is the same idea for failures: a trap that cost someone a debugging
session becomes a searchable file instead of folklore. Together they make the *current*
design auditable — not just "the tests pass today," but "here is the trail of reasoning,
including the dead ends, that led to today." That is exactly the property a simulator meant
to be *trusted* for educational use needs; a numerically plausible wrong answer is the
failure mode both mechanisms exist to make traceable.

## The sibling repo's own convention — and why it isn't merged

An internal validation-experiment archive maintained alongside this project keeps its own,
separate journal and gotchas conventions (its own gotchas notes — e.g. one on the ngspice
XSPICE codemodel needing `MFBINIT` set in its environment, which this repo's docs cite — and
its own journal). The two are deliberately **not** merged:
different repos, different scope. This repo is the simulator; that one is where it gets
exercised against Xyce/ngspice baselines, benchmarked, and written up as worked examples
(`experiments/`). A bug found *there* gets recorded *there*; a design consequence for *this*
repo gets a journal entry *here* that links across — the `crate-cscript-ffi.md` XSPICE
reference and the PFC anti-windup case study are exactly that pattern.

## Source material this was adapted from

- `docs/journal/README.md` — ported directly.
- `docs/journal/2026-08.md`, `docs/journal/2026-09.md` — the observed entry format and the
  example entries cited.
- `docs/gotchas/INDEX.md`, `docs/gotchas/_TEMPLATE.md`,
  `docs/gotchas/GOTCHA-001-cscript-ffi-fixture-race.md`.
- `AGENTS.md` — "Journal and gotchas", "Verification discipline".
- `README.md` — "Development".
