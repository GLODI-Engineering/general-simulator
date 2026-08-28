---
id: GOTCHA-001
discovered: 2026-08-29
discovered_by: contributor
scope:
  - crates/cscript-ffi/tests/**
severity: low
status: open
reproducibility: intermittent
tags: [testing, parallelism, cscript-ffi]
---

# GOTCHA-001 — `cargo test -p cscript-ffi` intermittently fails with `dlopen failed`

## Symptom

Re-running `cargo test -p cscript-ffi` (or `cargo test --workspace`) occasionally fails a
different subset of tests each time with a panic like:

```
thread 'stateful_fixture_accumulates_across_calls' panicked at crates/cscript-ffi/tests/fixtures.rs:53:10:
load accumulator fixture: Load { path: "/tmp/cscript-ffi-test-fixtures/libaccumulator.so", message: "dlopen failed" }
```

Out of 4 consecutive runs during a verification pass, 1 failed 3 of 15 tests; the other 3 runs
passed all 15. The failing subset differs each time, always among tests that load
`libaccumulator.so`.

## Reproduction

```bash
for i in 1 2 3 4 5; do cargo test -p cscript-ffi; done
```
No special circuit or netlist needed — this is a test-harness issue, not a simulator bug.

## Root cause

`crates/cscript-ffi/tests/fixtures.rs`'s `compile_fixture` writes every compiled `.so` to one
shared, non-unique path: `std::env::temp_dir().join("cscript-ffi-test-fixtures")`. Rust's test
harness runs tests in parallel by default; two tests that both use the same fixture name (e.g.
`stateful_fixture_accumulates_across_calls` and
`two_instances_of_the_same_library_keep_independent_state`, both using `libaccumulator.so`)
race: one thread's `cc -shared -o libaccumulator.so ...` (which truncates the file before
writing) can be mid-write exactly when another thread's `dlopen` reads the same path, producing
a nondeterministic `dlopen failed`.

## Fix or workaround

Not yet fixed. Workaround: `cargo test -p cscript-ffi -- --test-threads=1` avoids the race
(slower, but deterministic). A real fix would give each test (or at least each distinct
fixture-consuming test) its own unique output directory (e.g. a `tempfile::tempdir()` per test,
or suffix the `.so` path with the test's own name/thread id) instead of one shared
`cscript-ffi-test-fixtures` directory.

## Prevention

Would be caught immediately by running the crate's own test suite a handful of times in CI
with default parallelism before merging any change to `tests/fixtures.rs`'s compile helper.

## References

- Found while verifying an unrelated SPICE raw-file-output feature (`general-simulator-cli`);
  confirmed via 4 repeated runs that the failure is present on code that feature never touches.

## History

- 2026-08-29: Discovered and root-caused during a routine pre-commit verification pass.
