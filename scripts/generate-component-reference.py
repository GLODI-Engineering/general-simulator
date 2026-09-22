#!/usr/bin/env python3
"""Generates book/user-guide/src/component-reference.md from structured doc comments.

Do not edit the generated file by hand -- edit the source doc comments instead and rerun this
script (see scripts/README or the pre-commit hook that runs it).

Primary source: general-mna's src/block_graph.rs (path-sibling repo) -- the netlist-facing
`BlockKind`/`GateBinding` component reference (Purpose/Library/Description/Parameters/Errors/
Netlist form/Example/References), one doc comment per component, marked by a `# <Title>` first
line.

Secondary source: any `<!-- doc-ref: KIND -->`-tagged doc comment found under this repo's own
crates/ (e.g. continuous-blocks' math/implementation detail) -- appended to the matching
component (matched by the `kind=KIND` token in that component's own Netlist form block) as an
"## Implementation notes" section, so the deeper math/derivation can live next to the actual
implementation instead of being duplicated into general-mna.

Shared preamble: the doc comment on general-mna's `BlockInstance` struct carries the rules
every `kind=` block obeys and no per-component entry repeats -- the generic field-parsing
errors (missing field, non-numeric field, unknown field with the accepted-key list) and the
`ic=` initial-condition contract. Everything from its `**Generic field-parsing errors**`
paragraph to the end of that comment is rendered once, as a "Rules every `kind=` block shares"
section ahead of the per-library entries.
"""
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
GENERAL_MNA_BLOCK_GRAPH = REPO_ROOT.parent / "general-mna" / "src" / "block_graph.rs"
CRATES_DIR = REPO_ROOT / "crates"
OUT_PATH = REPO_ROOT / "book" / "user-guide" / "src" / "component-reference.md"

COMPONENT_MARKER_RE = re.compile(r"^\s*///\s*<!--\s*component\s*-->\s*$")
TITLE_RE = re.compile(r"^\s*///\s*#\s+(.+)$")
DOC_LINE_RE = re.compile(r"^\s*///(?: (.*)|)$")
DOC_REF_RE = re.compile(r"<!--\s*doc-ref:\s*([a-zA-Z0-9_]+)\s*-->")
KIND_TOKEN_RE = re.compile(r"\bkind=([a-zA-Z0-9_]+)\b")
BLOCK_INSTANCE_STRUCT_RE = re.compile(r"^\s*pub struct BlockInstance\b")
SHARED_RULES_START_RE = re.compile(r"^\*\*Generic field-parsing errors\*\*")


def strip_doc_prefix(line: str) -> str:
    m = DOC_LINE_RE.match(line)
    return m.group(1) if m and m.group(1) is not None else ""


def extract_entries(path: Path):
    """Yields (title, body_lines) for every doc-comment block marked with a leading
    `/// <!-- component -->` sentinel line, immediately followed by a `/// # Title` heading.
    The sentinel disambiguates a genuine component entry from an ordinary rustdoc `# Panics`/
    `# Examples`/etc. section header used elsewhere in the same file."""
    lines = path.read_text().splitlines()
    i = 0
    n = len(lines)
    while i < n:
        if not COMPONENT_MARKER_RE.match(lines[i]):
            i += 1
            continue
        i += 1
        if i >= n or not TITLE_RE.match(lines[i]):
            continue
        title = TITLE_RE.match(lines[i]).group(1).strip()
        body = [strip_doc_prefix(lines[i])]
        i += 1
        while i < n and DOC_LINE_RE.match(lines[i]):
            body.append(strip_doc_prefix(lines[i]))
            i += 1
        yield title, body


def extract_shared_rules(path: Path):
    """Returns the markdown lines of `BlockInstance`'s doc comment from its
    `**Generic field-parsing errors**` paragraph to the end of the comment -- the rules shared
    by every `kind=` block (generic errors, `ic=`), documented once there and nowhere else.
    Returns [] if the struct or the paragraph is missing, so a general-mna checkout predating
    that paragraph still generates the per-component entries."""
    lines = path.read_text().splitlines()
    struct_idx = next((i for i, l in enumerate(lines) if BLOCK_INSTANCE_STRUCT_RE.match(l)), None)
    if struct_idx is None:
        return []
    # Walk back over the attributes and the contiguous doc comment above the struct.
    end = struct_idx
    while end > 0 and lines[end - 1].lstrip().startswith("#["):
        end -= 1
    start = end
    while start > 0 and DOC_LINE_RE.match(lines[start - 1]):
        start -= 1
    body = [strip_doc_prefix(l) for l in lines[start:end]]
    first = next((i for i, l in enumerate(body) if SHARED_RULES_START_RE.match(l)), None)
    if first is None:
        return []
    return body[first:]


def extract_doc_refs(crates_dir: Path):
    """Returns {kind_key: [markdown_lines]} for every `<!-- doc-ref: KIND -->`-tagged doc block."""
    refs = {}
    for rs_file in crates_dir.rglob("*.rs"):
        lines = rs_file.read_text().splitlines()
        i = 0
        n = len(lines)
        while i < n:
            m = DOC_REF_RE.search(lines[i])
            if m and DOC_LINE_RE.match(lines[i]):
                key = m.group(1)
                body = []
                i += 1
                while i < n and DOC_LINE_RE.match(lines[i]):
                    body.append(strip_doc_prefix(lines[i]))
                    i += 1
                refs.setdefault(key, []).extend(body)
                continue
            i += 1
    return refs


def kind_key_for(body_lines):
    text = "\n".join(body_lines)
    m = KIND_TOKEN_RE.search(text)
    return m.group(1) if m else None


def library_of(body_lines):
    for line in body_lines:
        m = re.match(r"\*\*Library:\*\*\s*(.+)", line)
        if m:
            return m.group(1).strip()
    return "(uncategorized)"


def render(entries, doc_refs, shared_rules):
    by_library = {}
    for title, body in entries:
        lib = library_of(body)
        by_library.setdefault(lib, []).append((title, body))

    out = []
    out.append("<!-- AUTO-GENERATED by scripts/generate-component-reference.py -->")
    out.append("<!-- Do not edit by hand -- edit the source doc comments and rerun the script. -->")
    out.append("")
    out.append("# Component Reference")
    out.append("")
    out.append(
        "One entry per netlist `kind=` component, generated directly from the doc comments on "
        "`general_mna::block_graph::BlockKind`/`GateBinding` (and, where present, matching "
        "implementation-detail doc comments in this repo's own crates)."
    )
    out.append("")
    if shared_rules:
        out.append("## Rules every `kind=` block shares")
        out.append("")
        out.append(
            "From the doc comment on `general_mna::block_graph::BlockInstance`; the entries "
            "below document only what is specific to each component and rely on this section "
            "for the rest."
        )
        out.append("")
        out.extend(shared_rules)
        out.append("")

    for lib in sorted(by_library):
        out.append(f"## {lib}")
        out.append("")
        for title, body in sorted(by_library[lib], key=lambda t: t[0]):
            out.append(f"### {title}")
            out.append("")
            # body[0] is the raw `# Title` line already reflected in the heading above. The
            # rest of body's own `## Description`/`## Parameters`/`## Errors`/`## Netlist
            # form`/`## Example` lines are field labels within one component entry, not real
            # sub-navigation -- rendered as headings (even demoted to ####), ~38 components x
            # 5-7 fields each produced a four-level-deep sidebar (part -> ## library -> ###
            # component -> #### field) that fought the book's own TOC more than it helped a
            # reader. Rendered as a bold paragraph label instead: same visual field boundary,
            # no heading-level nesting, no numbered-outline explosion.
            for line in body[1:]:
                m = re.match(r"^##(?!#)\s*(.+)$", line)
                out.append(f"**{m.group(1).strip()}**" if m else line)
            key = kind_key_for(body)
            if key and key in doc_refs:
                out.append("")
                out.append("**Implementation notes**")
                out.append("")
                out.extend(doc_refs[key])
            out.append("")
    return "\n".join(out).rstrip() + "\n"


def main():
    if not GENERAL_MNA_BLOCK_GRAPH.exists():
        print(f"error: {GENERAL_MNA_BLOCK_GRAPH} not found (expects general-mna as a sibling directory)",
              file=sys.stderr)
        return 1

    entries = list(extract_entries(GENERAL_MNA_BLOCK_GRAPH))
    if not entries:
        print("error: no `# Title`-marked doc comments found in block_graph.rs", file=sys.stderr)
        return 1

    doc_refs = extract_doc_refs(CRATES_DIR)
    shared_rules = extract_shared_rules(GENERAL_MNA_BLOCK_GRAPH)
    if not shared_rules:
        print("warning: BlockInstance's `**Generic field-parsing errors**` paragraph not found; "
              "the shared-rules section is omitted", file=sys.stderr)
    OUT_PATH.parent.mkdir(parents=True, exist_ok=True)
    OUT_PATH.write_text(render(entries, doc_refs, shared_rules))
    print(f"wrote {OUT_PATH} ({len(entries)} component(s)"
          f"{', shared rules included' if shared_rules else ''})")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
