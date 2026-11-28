#!/usr/bin/env bash
# Fails *before cargo is reached* when a sibling repository's checkout declares a version this
# workspace's manifests cannot accept, and says which of the two possible reasons it is.
#
# general-simulator#142: CI checks out `spice-lsp`, `general-mna` and `gs-waveform-measurements`
# beside this repository and prefers a sibling branch of this run's own name.
# `general-mna`'s CI does the same in the other direction, so the resolution is **two-way** and
# no push order satisfies both: on a release pair, whichever branch is pushed first starts a run
# while its sibling branch does not yet exist, resolves to the sibling's default branch, and
# fails. What it used to fail with was
#
#     error: failed to select a version for the requirement `general-mna = "^0.11.0"`
#     location searched: /home/runner/work/general-simulator/general-simulator/general-mna
#
# on three of four jobs, including the Windows smoke test, on a branch where nothing was wrong.
# The waste is not the rerun, it is the diagnosis: that message names a requirement and a
# directory, and connecting it to "the other repository's branch is not pushed yet" takes
# knowing that the sibling-checkout step exists -- which a release is exactly when nobody has in
# mind.
#
# Only cross-repository dependencies are checked (`path` values that climb out of this
# repository, `../../../`): the in-workspace ones cannot disagree, because one commit moves them
# all.
#
# Usage: check-sibling-versions.sh [<this run's ref, for the hint>]
set -euo pipefail

ref="${1:-}"
problems=0
checked=0
# Every failure is buffered and printed after the table rather than interleaved with it: a log
# whose error sits between two "satisfies" lines is a log whose error gets scrolled past.
report=""

# Cargo's caret rule, which is what a bare `version = "0.11.0"` means: the leftmost non-zero
# component must match exactly and the rest must not be lower. So ^0.11.0 admits 0.11.3 and
# rejects 0.12.0 -- the case this script exists for, since every crate here is still 0.x.
caret_ok() {
  awk -v req="$1" -v got="$2" 'BEGIN {
    split(req, r, "."); split(got, g, ".");
    for (i = 1; i <= 3; i++) {
      if (r[i] == "") r[i] = 0; if (g[i] == "") g[i] = 0;
      sub(/[^0-9].*$/, "", r[i]); sub(/[^0-9].*$/, "", g[i]);
      r[i] += 0; g[i] += 0;
    }
    if (r[1] > 0)      ok = (g[1] == r[1]) && (g[2] > r[2] || (g[2] == r[2] && g[3] >= r[3]));
    else if (r[2] > 0) ok = (g[1] == 0) && (g[2] == r[2]) && (g[3] >= r[3]);
    else               ok = (g[1] == 0) && (g[2] == 0) && (g[3] == r[3]);
    exit ok ? 0 : 1;
  }'
}

# The version one manifest declares for itself, from its `[package]` section only -- a
# `[dependencies]` entry further down the file must not be mistaken for it.
declared_version() {
  sed -n '/^\[package\]/,/^\[[^p]/p' "$1" | grep -E '^version *=' | head -1 |
    sed -E 's/^version *= *"([^"]+)".*/\1/'
}

echo "Cross-repository version requirements"

while IFS= read -r manifest; do
  while IFS= read -r line; do
    name="$(printf '%s' "$line" | sed -E 's/^([A-Za-z0-9_-]+) *=.*/\1/')"
    dep_path="$(printf '%s' "$line" | sed -E 's/.*path *= *"([^"]+)".*/\1/')"
    want="$(printf '%s' "$line" | sed -E 's/.*version *= *"([^"]+)".*/\1/')"
    dep_dir="$(cd "$(dirname "$manifest")" && cd "$dep_path" 2>/dev/null && pwd || true)"

    if [ -z "$dep_dir" ] || [ ! -f "$dep_dir/Cargo.toml" ]; then
      printf '  %-26s %-10s requires %-10s NOTHING IS CHECKED OUT at %s\n' \
        "$name" "--" "$want" "$dep_path"
      report="$report
::error::$name is required by $manifest and nothing is checked out at $dep_path
  The workflow's checkout step for that sibling did not run, or ran into a different path.
"
      problems=$((problems + 1))
      continue
    fi

    have="$(declared_version "$dep_dir/Cargo.toml")"
    checked=$((checked + 1))

    # Anything but a bare caret requirement (`=`, `>=`, `~`, `*`, a range) is left to cargo
    # rather than guessed at here: a wrong answer from this script would be worse than the
    # message it replaces.
    case "$want" in
      [0-9]*.[0-9]*|[0-9]*.[0-9]*.[0-9]*) ;;
      *)
        printf '  %-26s %-10s requires %-10s (not a bare caret requirement -- left to cargo)\n' \
          "$name" "$have" "$want"
        continue
        ;;
    esac

    if caret_ok "$want" "$have"; then
      printf '  %-26s %-10s satisfies %s\n' "$name" "$have" "^$want"
      continue
    fi

    problems=$((problems + 1))
    printf '  %-26s %-10s requires %-10s DOES NOT SATISFY IT\n' "$name" "$have" "^$want"
    on="$(git -C "$dep_dir" branch --show-current 2>/dev/null || echo '')"
    at="$(git -C "$dep_dir" rev-parse --short HEAD 2>/dev/null || echo 'unknown commit')"
    why="  The two manifests disagree. During a release that usually means the sibling's own
  release commit has not reached its default branch yet, and the fix is to merge it there and
  re-run; otherwise they really do disagree and one of them has to move."
    if [ -n "$ref" ] && [ -n "$on" ]; then
      why="  That checkout is on '$on', not on a branch named '$ref'. If you are cutting a
  release pair, the sibling's matching branch is not pushed yet: push '$ref' in
  $(basename "${dep_dir%/servers/core}"), then re-run the failed jobs
  (gh run rerun <id> --failed). Nothing is wrong with the code in this repository.

  If '$ref' *is* pushed there, then the two manifests really disagree and one of them has
  to move: that is a real failure, not an ordering one."
    fi
    report="$report
::error::$name is checked out at $have, and this workspace requires ^$want
  $manifest
      requires  $name = \"$want\"
  $dep_dir/Cargo.toml
      declares  version = \"$have\"   (${on:-detached HEAD} @ $at)

$why
"
  done < <(grep -hoE '^[A-Za-z0-9_-]+ *= *\{[^}]*path *= *"(\.\./){2,}[^"]*"[^}]*version *= *"[^"]*"[^}]*\}' "$manifest" || true)
done < <(find . -name Cargo.toml -not -path './target/*' -not -path './*/target/*' | sort)

if [ "$problems" -gt 0 ]; then
  printf '%s\n' "$report"
  echo "$problems cross-repository requirement(s) cannot be satisfied by the checkouts in this run." >&2
  exit 1
fi
echo "  all $checked cross-repository requirement(s) satisfied"
