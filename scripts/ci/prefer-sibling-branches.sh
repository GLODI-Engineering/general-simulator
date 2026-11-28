#!/usr/bin/env bash
# Checks out each sibling repository at a branch of this run's own name when it has one, and
# says -- for every sibling, including the ones that did not move -- which ref it ended up on.
#
# Why the preference exists: a change that moves a version across a repository boundary cannot
# be green in either repository until it is green in both. This workspace asks for a sibling
# version that exists only on the sibling's own branch, so without this step the two
# repositories deadlock and neither side can go first.
#
# Why it reports: the resolution is **two-way** (general-mna's CI checks this repository out the
# same way), so on a release pair whichever branch is pushed first starts a run while its
# sibling does not yet exist. That run resolves to the default branch and fails in `cargo`,
# which says nothing about why -- general-simulator#142. A reader of the log has to be able to
# tell "the matching branch is not pushed yet" from "the code is broken" without already
# knowing this step is here, so the ref each sibling is on is printed whether it matched or not,
# and `check-sibling-versions.sh` turns the mismatch it causes into its own error.
#
# Usage: prefer-sibling-branches.sh <ref> <dir>...
set -euo pipefail

ref="${1:?usage: prefer-sibling-branches.sh <ref> <dir>...}"
shift
[ "$#" -gt 0 ] || { echo "no sibling directories given" >&2; exit 2; }

echo "Sibling checkouts (this run's ref: '$ref')"

for d in "$@"; do
  if [ ! -d "$d" ]; then
    echo "  $d: NOT CHECKED OUT -- the workflow's checkout step for it did not run"
    continue
  fi
  if git -C "$d" ls-remote --exit-code --heads origin "$ref" >/dev/null 2>&1; then
    git -C "$d" fetch -q --depth 1 origin "$ref"
    git -C "$d" checkout -q FETCH_HEAD
    printf '  %-26s branch %-34s %s  <- matched this run'\''s ref\n' \
      "$d" "'$ref'" "$(git -C "$d" rev-parse --short HEAD)"
  else
    # Whatever the checkout step cloned, which on a runner is that repository's default
    # branch. Reported as the branch it is rather than as "the default branch", so the line
    # stays true when this script is run against a local checkout that is on something else.
    here="$(git -C "$d" branch --show-current 2>/dev/null || echo '(detached)')"
    printf '  %-26s branch %-34s %s  <- as checked out; it has no branch '\''%s'\''\n' \
      "$d" "'$here'" "$(git -C "$d" rev-parse --short HEAD)" "$ref"
  fi
done
