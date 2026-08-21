# Documentation

Two separate [mdBook](https://rust-lang.github.io/mdBook/)s, one per audience:

- **`user-guide/`** — for anyone writing a netlist or wiring up an experiment: installation,
  the netlist/device-file grammar, the block library, CLI reference, worked examples.
- **`dev-guide/`** — for contributors, reviewers, or anyone who wants the math and the
  rationale: the LCP/DAE formulation, the switch model and why it's built the way it is, the
  block graph's causality/cycle-detection design, a crate-by-crate tour, and contributing
  conventions.

Every chapter file currently in `src/` is a **skeleton**: a title plus an outline of what
belongs there and pointers to the existing source material (module doc comments, `AGENTS.md`,
`docs/architecture.md`, the journal, and — for several chapters — this project's own
documentation-planning conversation, which already drafted some sections close to publication
quality). Fill these in incrementally; each is independently buildable and reviewable.

## Building the web version

```bash
cargo install mdbook mdbook-katex   # one-time
cd user-guide && mdbook build       # -> user-guide/book/ (gitignored, regenerate as needed)
cd ../dev-guide && mdbook build     # -> dev-guide/book/
```

`mdbook serve` (run from either book's own directory) live-reloads at `http://localhost:3000`
while editing.

Math uses [KaTeX](https://katex.org/) syntax (`$inline$`, `$$display$$`) via the
`mdbook-katex` preprocessor, already configured in both `book.toml`s.

## Building the PDF version

```bash
./build-pdf.sh user-guide   # -> book/user-guide.pdf
./build-pdf.sh dev-guide    # -> book/dev-guide.pdf
```

Requires `pandoc` plus a LaTeX engine (`xelatex` recommended); see `build-pdf.sh`'s own header
for exact package names. Both the web and PDF outputs render the *same* `.md` sources in
`src/` — there is no separate PDF-only source to keep in sync.

## Publishing

The `.github/workflows/docs.yml` workflow builds both books' HTML, the PDFs, and `cargo doc`
(the API reference — kept separate from these books, linked to from the dev guide's crate
tour rather than duplicated), and deploys everything to GitHub Pages on push to `main`:

- `/user-guide/` — the user guide
- `/dev-guide/` — the developer guide
- `/api/` — `cargo doc` output (superseded by docs.rs automatically once this crate is
  published to crates.io — keep the CI-built copy only until then, or as a version-pinned
  archive after)
- `*.pdf` — both PDFs, linked from the site's landing page

Before enabling the workflow: replace `REPLACE_ME` in both `book.toml`s'
`git-repository-url`/`edit-url-template` with the real GitHub org/repo once it's public.
