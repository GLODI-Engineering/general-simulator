---
id: GOTCHA-006
discovered: 2026-11-27
discovered_by: agent
scope:
  - .github/workflows/ci.yml
  - .github/workflows/release.yml
  - scripts/ci/**
severity: medium
status: documented
reproducibility: deterministic
tags: [ci, release, general-mna, cross-repository]
---

# GOTCHA-006 — a release pair's first CI run always fails, in whichever repository was pushed first

## Symptom

On a release branch — a branch that moves this workspace's version and `general-mna`'s together
— three of four jobs go red, the Windows smoke test among them, on a branch where nothing is
wrong:

```
error: failed to select a version for the requirement `general-mna = "^0.11.0"`
location searched: /home/runner/work/general-simulator/general-simulator/general-mna
```

`gh run rerun <id> --failed` then passes every job, with no change to any file.

## Cause

CI checks out `spice-lsp`, `general-mna` and `gs-waveform-measurements` beside this repository
and prefers a sibling branch **of this run's own name**, falling back to the sibling's default
branch. That preference exists because a change crossing a repository boundary cannot be green
in either repository until it is green in both.

`general-mna`'s CI does exactly the same in the other direction. So the resolution is **two-way,
and no push order satisfies both**: whichever branch is pushed first starts a run while its
sibling branch does not yet exist, resolves to the sibling's default branch — which still
declares the old version — and fails in cargo's resolver.

This is ordering, not breakage. It is not avoidable by pushing in a particular order, which is
the part that costs time: the natural response to a red Windows job on a release branch is to go
looking for a Windows problem.

## What to do

Push both branches, then `gh run rerun <id> --failed` on whichever started first. Nothing is
wrong with the code.

## What changed, so the next person does not have to know this

[#142](https://github.com/GLODI-Engineering/general-simulator/issues/142). The failure still
happens — it is inherent in a two-way preference — but it now says so itself, before cargo is
reached:

- `scripts/ci/prefer-sibling-branches.sh` prints, for **every** sibling and whether it matched
  or not, which ref that checkout ended up on and at which commit;
- `scripts/ci/check-sibling-versions.sh` compares every cross-repository `path` + `version`
  dependency against the version the checkout actually declares, and on a mismatch names both
  versions, both manifests, the branch it expected, and which of the two possible causes it is:
  *the sibling's matching branch is not pushed yet* or *the two manifests really disagree*.

It runs in all three sibling-using CI jobs and in `release.yml`'s build matrix, where it also
spares four platform builds — two of them billed — from ending in a resolver error.

## Related

- [#142](https://github.com/GLODI-Engineering/general-simulator/issues/142) is this trap.
- [#143](https://github.com/GLODI-Engineering/general-simulator/issues/143) is the other half of
  the same release friction: the workflow could not create the release it uploaded to.
- `general-mna` must be bumped whenever this workspace is, and its *own* version, not only its
  pins — which is why a release is a pair of branches in the first place.
