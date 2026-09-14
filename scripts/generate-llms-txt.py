#!/usr/bin/env python3
"""Generates book/llms.txt and book/llms-full.txt from both mdBooks' real chapter content.

Do not edit the generated files by hand -- edit the chapter .md files (or their SUMMARY.md
ordering) instead and rerun this script.

llms.txt follows the https://llmstxt.org convention: a short, curated Markdown index (title,
one-line project summary, then one link + one-line description per chapter, grouped by the
same Part headings SUMMARY.md already uses) meant to fit easily in an LLM's context instead of
making it crawl rendered HTML with nav/CSS/JS noise. llms-full.txt is the same convention's
fuller sibling -- every chapter's actual content concatenated in full, for a reader (human or
model) who wants the whole book as one plain-text file rather than an index to follow.

Both walk SUMMARY.md top to bottom the same way book/build-pdf.sh already does, so chapter
order and Part grouping never drift out of sync between the PDF, llms.txt, and llms-full.txt.

Links point at raw.githubusercontent.com (the plain .md source, not the rendered HTML page) --
the llms.txt convention's own stated preference, since a model fetching a link benefits from
plain Markdown far more than from HTML built for a browser.
"""
import re
import sys
from pathlib import Path

BOOK_DIR = Path(__file__).resolve().parent.parent / "book"
RAW_BASE = "https://raw.githubusercontent.com/GLODI-Engineering/general-simulator/main"

BOOKS = [
    ("user-guide", "User Guide", "For netlist authors: writing, running, and reading the output of a general-simulator netlist."),
    ("dev-guide", "Developer Guide", "For contributors: the math, internal mechanism, and rationale behind every major design choice."),
]

PROJECT_SUMMARY = (
    "An open-source Rust simulator for piecewise-linear (PWL) circuits and mixed "
    "circuit/block-diagram systems. Resolves which segment every switching device is in as a "
    "Linear Complementarity Problem each timestep instead of Newton-Raphson + voltage "
    "limiting, and provides native control-theory blocks (transfer function, state-space, "
    "PID, discrete-time variants, coordinate transforms) so a digital control loop can be "
    "built directly in the netlist and validated against its own real, switching power stage."
)


def parse_summary(book_name):
    """Returns a list of (part_title_or_None, chapter_title, rel_path) in SUMMARY.md order."""
    summary = BOOK_DIR / book_name / "src" / "SUMMARY.md"
    entries = []
    current_part = None
    for line in summary.read_text().splitlines():
        m = re.match(r"^-?\s*\[(.+?)\]\((.+?\.md)\)", line.strip())
        if m:
            entries.append((current_part, m.group(1), m.group(2)))
            continue
        m = re.match(r"^#\s+(.+)$", line)
        if m and line.strip() != "# Summary":
            current_part = m.group(1).strip()
    return entries


def first_description_sentence(text):
    """Pulls a one-line description out of a chapter's body: the first sentence of its first
    real prose paragraph, with the leading H1 stripped, fenced code blocks and sub-headings
    skipped over entirely (not treated as a stopping point until a paragraph has actually been
    found), and Markdown emphasis/links/code-spans flattened to plain text so it reads cleanly
    as a single index line."""
    lines = text.splitlines()
    # Skip the H1 title line and any immediately-following blank lines.
    body = lines[1:] if lines and lines[0].startswith("# ") else lines
    paragraph = []
    in_code_fence = False
    for line in body:
        stripped = line.strip()
        if stripped.startswith("```"):
            in_code_fence = not in_code_fence
            continue
        if in_code_fence:
            continue
        if not stripped:
            if paragraph:
                break
            continue
        if stripped.startswith("#") or stripped.startswith(">"):
            # A sub-heading or blockquote before any prose has been collected isn't a stopping
            # point -- keep scanning for the chapter's actual first paragraph past it.
            continue
        paragraph.append(stripped)
    text = " ".join(paragraph)
    text = re.sub(r"`([^`]+)`", r"\1", text)  # `code` -> code
    text = re.sub(r"\*\*([^*]+)\*\*", r"\1", text)  # **bold** -> bold
    text = re.sub(r"\[([^\]]+)\]\([^)]+\)", r"\1", text)  # [text](url) -> text
    text = re.sub(r"\$([^$]+)\$", r"\1", text)  # $math$ -> math
    # First sentence only (first ". " outside the obviously-abbreviation-free common case),
    # capped so one wandering sentence can't blow out the index line.
    m = re.search(r"^(.{20,220}?[.!?])(\s|$)", text)
    sentence = m.group(1) if m else text[:220]
    return sentence.strip()


def build_llms_txt():
    out = [f"# general-simulator\n", f"> {PROJECT_SUMMARY}\n"]
    for book_name, book_title, book_blurb in BOOKS:
        out.append(f"## {book_title}\n")
        out.append(f"{book_blurb}\n")
        last_part = None
        for part, title, rel_path in parse_summary(book_name):
            if part and part != last_part:
                out.append(f"\n### {part}\n")
                last_part = part
            chapter_file = BOOK_DIR / book_name / "src" / rel_path
            desc = first_description_sentence(chapter_file.read_text())
            url = f"{RAW_BASE}/book/{book_name}/src/{rel_path}"
            out.append(f"- [{title}]({url}): {desc}")
        out.append("")
    return "\n".join(out) + "\n"


def build_llms_full_txt():
    out = [f"# general-simulator\n", f"> {PROJECT_SUMMARY}\n"]
    for book_name, book_title, _ in BOOKS:
        out.append(f"\n{'=' * 80}\n# {book_title}\n{'=' * 80}\n")
        last_part = None
        for part, title, rel_path in parse_summary(book_name):
            if part and part != last_part:
                out.append(f"\n## {part}\n")
                last_part = part
            chapter_file = BOOK_DIR / book_name / "src" / rel_path
            out.append(f"\n<!-- {book_title} / {title} -->\n")
            out.append(chapter_file.read_text().rstrip())
            out.append("")
    return "\n".join(out) + "\n"


def main():
    llms_txt = build_llms_txt()
    llms_full_txt = build_llms_full_txt()
    (BOOK_DIR / "llms.txt").write_text(llms_txt)
    (BOOK_DIR / "llms-full.txt").write_text(llms_full_txt)
    print(f"wrote {BOOK_DIR / 'llms.txt'} ({len(llms_txt)} bytes)")
    print(f"wrote {BOOK_DIR / 'llms-full.txt'} ({len(llms_full_txt)} bytes)")


if __name__ == "__main__":
    sys.exit(main())
