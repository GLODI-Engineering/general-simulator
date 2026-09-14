#!/usr/bin/env bash
# Builds a single PDF from one mdBook's SUMMARY.md order, via Pandoc + a LaTeX engine.
#
# Why Pandoc instead of mdbook-pdf (a headless-Chrome print of the rendered HTML): this book
# leans on real math (the LCP/DAE derivations in the dev guide), and Pandoc's LaTeX backend
# gives genuinely typeset math and print typography instead of a printed webpage. The dollar-
# delimited math syntax already used for mdbook-katex ($...$/$$...$$) is exactly what Pandoc's
# own Markdown reader expects, so no source duplication is needed between the web and PDF
# outputs -- both render the same .md files.
#
# Usage:
#   book/build-pdf.sh user-guide
#   book/build-pdf.sh dev-guide
#
# Requires: pandoc, and a LaTeX engine (xelatex recommended -- handles Unicode in the source
# prose better than pdflatex; this repo's own docs use real em/en dashes and non-ASCII
# characters throughout). Rendered with the DejaVu family (set via -V mainfont/sansfont/
# monofont below) instead of xelatex's Latin Modern default, because Latin Modern is missing
# glyphs this repo's docs actually use -- box-drawing characters in ASCII diagrams (crate-tour.md)
# and Unicode math symbols in prose (>=, ~=, =>) -- which silently render as blank boxes
# otherwise (xelatex only warns, it doesn't fail the build). On Debian/Ubuntu:
# `sudo apt-get install -y pandoc texlive-xetex texlive-latex-extra fonts-dejavu`.
# pdf-header.tex (below, via --include-in-header) needs the `seqsplit` and `fvextra` packages
# to let long unbroken inline code (file paths, function names) and long code-block lines wrap
# instead of overflowing the page margin -- both ship in texlive-latex-extra, no extra install.
set -euo pipefail

if [ $# -ne 1 ] || { [ "$1" != "user-guide" ] && [ "$1" != "dev-guide" ]; }; then
    echo "usage: $0 {user-guide|dev-guide}" >&2
    exit 1
fi

book_name="$1"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
book_dir="$script_dir/$book_name"
src_dir="$book_dir/src"
summary="$src_dir/SUMMARY.md"

if ! command -v pandoc >/dev/null 2>&1; then
    echo "error: pandoc not found on PATH -- see this script's own header for install instructions" >&2
    exit 1
fi

title="$(grep -m1 '^title *= *"' "$book_dir/book.toml" | sed -E 's/^title *= *"(.*)"$/\1/')"

work_dir="$(mktemp -d)"
trap 'rm -rf "$work_dir"' EXIT
combined="$work_dir/combined.md"
: > "$combined"

# Walks SUMMARY.md top to bottom, in the exact order mdBook itself renders the nav: a bare
# "# Part Title" line becomes a part-divider heading, a "[Chapter](file.md)" link appends that
# chapter's own file content, in place, at its own heading level (every chapter file already
# starts with a single top-level "# Title" matching its SUMMARY.md entry, so no heading-level
# rewriting is needed here).
while IFS= read -r line; do
    if [[ "$line" =~ ^\[.*\]\((.*\.md)\)$ ]] || [[ "$line" =~ \[.*\]\((.*\.md)\) ]]; then
        rel_path="${BASH_REMATCH[1]}"
        chapter_file="$src_dir/$rel_path"
        if [ -f "$chapter_file" ]; then
            cat "$chapter_file" >> "$combined"
            printf '\n\n' >> "$combined"
        else
            echo "warning: SUMMARY.md references missing file: $rel_path" >&2
        fi
    elif [[ "$line" =~ ^#\ +(.+)$ ]] && [[ "$line" != "# Summary" ]]; then
        part_title="${BASH_REMATCH[1]}"
        printf '# %s\n\n' "$part_title" >> "$combined"
    fi
done < "$summary"

# Chapters reference images relative to their OWN directory (a top-level chapter uses
# `images/foo.png`, an `examples/*.md` chapter uses `../images/foo.png`) -- but $combined lives
# in a scratch dir, so pandoc can't resolve either as a literal relative path from there.
# --resource-path is pandoc's search-path option for exactly this: each entry is tried in turn
# as a base to resolve a relative image path against, so listing both $src_dir (resolves
# `images/foo.png`) and every subdirectory (resolves `../images/foo.png`, since
# `$src_dir/examples/../images/foo.png` = `$src_dir/images/foo.png`) covers every chapter depth
# actually used without hand-listing directories.
resource_path="$src_dir"
while IFS= read -r -d '' subdir; do
    resource_path="$resource_path:$subdir"
done < <(find "$src_dir" -mindepth 1 -type d -print0)

# A dense page (running text + a big table + prose packed right up against \textheight) left
# essentially no gap between the last line of content and the page-number footer -- visually
# read as the two overlapping. TeX's page-breaking always fills a page up to \textheight
# before breaking, so the last line of ANY full page sits close to that boundary regardless of
# how big \textheight is -- shrinking \textheight (a plain bigger `bottom` margin) does nothing
# by itself. What actually creates the gap is `footskip` (the geometry package's own distance
# from \textheight's bottom edge to the footer baseline) -- confirmed the hard way, an initial
# footskip=0.5in produced no visible change (apparently close to geometry's own default) before
# a much larger test value proved footskip was the right knob at all. `bottom` is widened too,
# giving geometry more total room to allocate between text and footer. This is a separate fix
# from pdf-header.tex's own float-placement fix above (that one stops a figure from being
# forced into too-small a remaining space; this one gives every full page, figures or not, a
# real gap above its footer).
out_pdf="$script_dir/${book_name}.pdf"
pandoc "$combined" \
    --from=markdown+smart \
    --pdf-engine=xelatex \
    --resource-path="$resource_path" \
    --toc \
    --toc-depth=2 \
    --number-sections \
    --include-in-header="$script_dir/pdf-header.tex" \
    -V title="$title" \
    -V geometry:margin=1in \
    -V geometry:bottom=1.4in \
    -V geometry:footskip=0.8in \
    -V colorlinks=true \
    -V linkcolor=blue \
    -V urlcolor=blue \
    -V mainfont="DejaVu Serif" \
    -V sansfont="DejaVu Sans" \
    -V monofont="DejaVu Sans Mono" \
    -o "$out_pdf"

echo "wrote $out_pdf"
