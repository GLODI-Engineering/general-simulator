---
name: write-component-doc
description: Write or update a netlist-component reference entry (a `BlockKind`/`GateBinding` variant, or a PWL device) for general-simulator's generated Component Reference. Use whenever adding a new `kind=` block, changing an existing one's fields/behavior, or filling in a component this reference doesn't cover yet.
---

# Write a component reference entry

This produces the source doc comments that `scripts/generate-component-reference.py`
compiles into `book/user-guide/src/component-reference.md` — real documentation generated
from code, the same way `cargo doc` is, not a hand-maintained document written once and left
to drift. Never edit `component-reference.md` directly; it is regenerated and any hand edit is
lost.

## The one non-negotiable rule: no invented references

**Never write a `## References` section unless the user explicitly asks for one for that
specific entry.** Citing a textbook, paper, or standard is exactly the kind of fact a model
confidently fabricates — a plausible-looking author, title, and year that doesn't correspond
to a real work. A missing citation costs nothing; a fabricated one silently corrupts a
reference document a reader will trust and act on. If you are not looking at the actual
publication (given by the user, or independently verifiable), do not cite it. When the user
does ask for a citation, prefer one already used elsewhere in this project's own docs
(`book/dev-guide/`, `docs/GRAMMAR.md`-style citations in `spice-lsp`) over recalling one from
memory, and say so if you cannot verify it rather than guessing.

Every other claim in an entry — parameter names, error conditions, netlist syntax — must be
grounded in code you actually read this session, not remembered or inferred from a similar
block. See "Grounding discipline" below.

## Where content lives (split by repo)

A component's netlist-facing shape and its underlying math/implementation live in different
repos; each gets its own half of the entry, and the generator merges them.

1. **Netlist-facing half — `general-mna/src/block_graph.rs`** (sibling repo, path dependency).
   This is the primary, required half: Purpose, Library, Description, Parameters, Errors,
   Netlist form, Example. Attach it to the `BlockKind`/`GateBinding` variant itself.
2. **Implementation-detail half — this repo's own crates** (`continuous-blocks`, `pwl-devices`,
   etc.), optional. Deeper math/derivation that belongs next to the actual implementation
   rather than duplicated into general-mna. Tag it so the generator finds it.

Never duplicate the same fact in both halves — if it's already accurate in one place, link to
it (`` `continuous_blocks::Pid`'s own doc comment `` ) instead of restating it.

## Template

On the `BlockKind`/`GateBinding` variant in `general-mna/src/block_graph.rs`:

```rust
/// <!-- component -->
/// # <Component Name>
/// **Purpose:** one line — what problem this block solves.
/// **Library:** <Category> / <Subcategory>  — e.g. "Control / Continuous", "Sources",
/// "Logic", "Power Electronics", "Machines", "Electrical Interface".
///
/// ## Description
/// What it computes, in prose. Real math as KaTeX (see below), not ASCII approximations.
/// Cross-reference the implementation half with a plain `` `crate::Type` `` mention (not an
/// intra-doc link, since it's in a different crate — see the existing entries for the
/// established phrasing).
///
/// ## Parameters
/// - `field=<type>` — meaning, units, constraints. One bullet per netlist field, in the order
///   the netlist line would give them.
///
/// ## Errors
/// - Every *component-specific* validation failure, with its real error text or enum variant —
///   not the generic missing-field/malformed-number error every `kind=` block already shares
///   (that's documented once, on `BlockInstance`; just point to it).
///
/// ## Netlist form
/// ```text
/// NAME kind=<kind> field=<type> ...
/// ```
///
/// ## Example
/// ```text
/// NAME1 kind=<kind> field=<concrete value> ...
/// ```
Variant {
    /// field docs — `missing_docs` requires these regardless of this template.
    field: Type,
},
```

The `<!-- component -->` sentinel line is required and must be the line immediately before
`# <Component Name>`. Without it the generator will not find the entry, **and** a bare
`# Title` line risks colliding with an ordinary rustdoc section header (`# Panics`,
`# Examples`) used elsewhere in the same file — the sentinel is what disambiguates a genuine
component entry from those.

`## References` is omitted from this template on purpose — see the rule above. If the user
asks for one, add it as the last section, after `## Example`.

On the matching implementation type in this repo's own crate (optional half):

```rust
/// <!-- doc-ref: <kind> -->
/// Implementation-level math/derivation detail...
```

`<kind>` must exactly match the `kind=` token used in the netlist-facing half's own "Netlist
form" block — that's how the generator matches the two halves together. Getting this wrong
means the implementation-detail half silently never appears in the generated output (the
generator does not error on an unmatched tag); always regenerate and check the entry actually
picked it up.

## Grouping: document at the type's own granularity, not per named function

If a `BlockKind` variant wraps a sub-enum of interchangeable named operations sharing one
input/output/error contract (e.g. `MathFn1` wrapping `cos`/`sin`/`exp`/`sqrt`/... — over twenty
single-argument real functions, all `Scalar -> Scalar` or elementwise `Vector -> Vector`, all
erroring identically on shape mismatch), write **one entry** for the variant describing the
shared contract, plus a compact `name -> formula` table listing every member — not one entry
per function. Before writing a new entry, check whether the variant already has this shape
(`grep -n "pub enum" crates/continuous-blocks/src/*.rs` and cross-check against the matching
`BlockKind` variant in general-mna) — collapsing after the fact is more work than starting
right. `MathFn1`, `MathFn2`, `MathFn3`, `LogicOp`, `FlipFlopKind`, and `CoordinateTransform` are
the currently-known cases of this pattern; there may be others among the components not yet
written.

## Math: real KaTeX, one source

Both `mdbook-katex` (HTML) and Pandoc+xelatex (`book/build-pdf.sh`, the PDF) already render
`$...$`/`$$...$$` — confirmed working, this is not new infrastructure (`book/dev-guide/src/lcp-formulation.md` already uses it). Write the formula once, as real KaTeX, in whichever half
of the entry it belongs to. Do not also write an ASCII-approximation version "for `cargo doc`
readability" — that's exactly the duplication-drift problem this whole generated-docs system
exists to avoid. `cargo doc` will show the literal `$...$` source, which stays readable as
plain text even unrendered; that's an acceptable, deliberate trade-off, not a bug to work
around.

## Netlist-grammar facts every entry needs to get right

Two facts, found the hard way by actually running the CLI against plausible-looking netlists
that turned out to be wrong — do not rediscover these:

- **No mandatory title line.** `general-mna`'s `build_system`/`parse_and_flatten` parse the
  netlist as a SPICE *fragment*, not a full document — the first line is a real element/block
  statement, not a skipped title. A netlist that leads with an ordinary SPICE title line (the
  normal convention, and what `general-mna`'s own README examples use via `build_document`)
  will have that title parsed as a bogus device/block declaration and fail. Every `.cir` fixture
  and every `## Example`/`## Netlist form` block starts directly with a real statement.
- **A field value containing whitespace must be `"double-quoted"`.** `general-spice-core`'s
  block-instance tokenizer splits on whitespace outside `"..."` spans (added this session,
  `spice-lsp/servers/core/src/parser/mod.rs`'s `split_respecting_double_quotes`) — this is
  relevant to any `lib=`/`path=` field whose value might be a path through a directory
  containing a space. State this explicitly in `## Parameters` for any such field, don't just
  assume a value never needs it.

## Grounding discipline

Every parameter, default, error message, and constraint must come from code read this
session — not from memory of a similar block, not from what "seems reasonable." Concretely,
before writing an entry:

1. Read the `BlockKind`/`GateBinding` variant's own current doc comment and field types in
   `general-mna/src/block_graph.rs`.
2. Read the matching `match kind { "..." => ... }` arm in `general-mna/src/system_builder.rs`
   — this is the actual netlist-field-parsing code, and the ground truth for "Netlist form" and
   component-specific "Errors" (required vs. optional fields, validation calls, exact error
   strings).
3. Read the underlying type's own constructor/validation in this repo's crate (e.g.
   `Pid::new`, `Vco::new`) for the real error enum/conditions — do not guess plausible-sounding
   ones.
4. Every "Example" netlist and every "Errors" claim must be verified by an actual run of the
   real simulator, not by inspection alone — see "Testing Examples and Errors against a real
   run" below. This is not optional and not satisfied by `cargo run --example` against
   `general_mna::build_system` alone (that only proves the netlist parses and builds a
   `System`; it doesn't prove the simulation actually runs to completion, or that a claimed
   error is the error the CLI actually produces) — use the full `general-simulator-cli` binary.

## Testing Examples and Errors against a real run

**This is mandatory practice, not an optional nicety — do not write or edit a component
reference doc comment without a passing `doc-verify/<kind>/test_<kind>.py` backing every
`## Example` and every `## Errors` claim in it.** A collaborator (human or agent) reading this
skill without having internalized that yet: treat the previous sentence as the actual
instruction, not color commentary.

Build the release CLI once per session — **with the `python` feature**, even if the component
you're writing doesn't need it, so the same build serves every other component's tests too
(debug is noticeably slower for repeated runs, and a `python`-less build makes any
`kind=pyblock`/`kind=pyfunc` test fail with `PythonSupportNotCompiledIn`, wasting time
rediscovering why):

```bash
cargo build --release -p general-simulator-cli --features python
```

### `doc-verify/<kind>/` is a committed, reviewable test folder — not scratch

One subfolder per component, named after its `kind=` token (e.g. `doc-verify/pid/`,
`doc-verify/cscript/`). **It is committed to the repo, not gitignored** (only build artifacts
compiled *from* its source — `.so`/`.dylib`/`.dll`/`__pycache__`/`.pyc` — are gitignored; see
`.gitignore`). Every one of its `.cir`/`.c`/`.py` files is real, checked-in source, reviewed
like any other change — this is the actual evidence a doc entry's claims are true, and the next
person to touch that component reruns it before changing anything.

Every folder needs, at minimum:

- **`README.md`** — what's in the folder and what each file proves. One line per fixture file
  naming which `## Example` or `## Errors` claim it verifies. Point at `test_<kind>.py` as how
  to actually run it, and note any build requirement (e.g. `--features python`) or any claim
  *not* covered by the automated test and why (see `doc-verify/pyblock/README.md` for a worked
  example of that last case — one error message needs a second, differently-featured build to
  reproduce, which isn't worth the orchestration cost to automate).
- **Commented `.cir` files.** Every fixture explains, in `*`-prefixed SPICE comments, what it's
  testing and what the expected result is — not just the bare netlist. A reader (or an agent
  six months from now) should be able to tell what a fixture proves without cross-referencing
  the doc comment.
- **`test_<kind>.py`** — one function per documented behavior (one per `## Example`, one per
  distinct `## Errors` claim), written like real unit tests: a docstring naming exactly which
  doc-comment claim it verifies, a real assertion on the actual output (not just "exit code
  0" — check the specific column/value the example claims), imports the shared
  `doc-verify/_lib.py` harness. Runnable standalone (`python3 doc-verify/<kind>/test_<kind>.py`)
  *and* via `pytest` (`python3 -m pytest doc-verify/<kind>`) — see any existing `test_*.py`
  (`doc-verify/pid/`, `doc-verify/cscript/`) for the exact pattern; copy its structure rather
  than inventing a new one.

`doc-verify/_lib.py` provides `run`/`run_transient`/`run_dc` (subprocess the real CLI, assert
success unless `expect_success=False`) and `parse_csv` (CLI stdout → list of dicts, values
coerced to `float`). Use it; don't hand-roll subprocess calls per component.

### Workflow

**Every Example** in an entry must be an actual run that completes successfully:

```bash
mkdir -p doc-verify/<kind>
# write doc-verify/<kind>/example.cir (commented!) and any .c/.py it needs — the *exact*
# netlist the doc comment's "## Example" section will contain, not a simplified stand-in
```

then a `test_<kind>.py` function that runs it via `_lib.run_transient` (or `run_dc`) and asserts
the specific expected value(s) — not just that it exits `0`. Confirm the test actually passes
before copying the netlist into the doc comment; never write a plausible-looking netlist and
assume it works.

**Every Errors claim** must be reproduced, not just cited from reading the code: a commented
`error_<condition>.cir` fixture in the same folder, and a `test_<kind>.py` function that runs it
with `expect_success=False` and asserts the *actual* observed stderr text — not a paraphrase,
and not the string literal read out of the source unless you've confirmed it's exactly what the
CLI prints (message formatting, `line N:` prefixes, and `{e:?}` `Debug` formatting of an inner
enum can all change the final text). A component-specific error that `system_builder.rs` rejects
at parse time (most of them) will fail before the simulation even starts — that's fine, it's
still a real, reproduced error, not a hypothetical one.

Do not skip this for an error that "obviously" reproduces from reading the code alone — the
whole point is to catch the cases where it doesn't (a message string that changed since the
doc comment you're citing was written, an error path that's actually unreachable, a validation
that happens later than expected, or — as happened this session — a claim that needs a
differently-configured build to reproduce at all).

## After writing an entry

```bash
python3 doc-verify/<kind>/test_<kind>.py           # must pass — this is the evidence
python3 scripts/generate-component-reference.py    # regenerate component-reference.md
```

Then, from the touched repo(s):

```bash
# general-mna
cargo doc --no-deps --lib   # must be 0 warnings (missing_docs, broken intra-doc links)
cargo test --all-targets

# general-simulator (whichever crate you touched, e.g. continuous-blocks)
cargo build

# general-simulator/book/user-guide — confirm it actually builds, not just parses
mdbook build
```

A `cargo doc` warning (missing doc on a field, a broken intra-doc link) or an `mdbook build`
failure means the entry is not done — fix it before moving to the next component, not in a
later cleanup pass.
