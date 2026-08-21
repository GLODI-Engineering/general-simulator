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
# characters throughout). On Debian/Ubuntu: `sudo apt-get install -y pandoc texlive-xetex
# texlive-latex-extra`.
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

out_pdf="$script_dir/${book_name}.pdf"
pandoc "$combined" \
    --from=markdown+smart \
    --pdf-engine=xelatex \
    --toc \
    --toc-depth=2 \
    --number-sections \
    -V title="$title" \
    -V geometry:margin=1in \
    -V colorlinks=true \
    -V linkcolor=blue \
    -V urlcolor=blue \
    -o "$out_pdf"

echo "wrote $out_pdf"
