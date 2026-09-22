---
id: GOTCHA-001
discovered: 2026-08-29
discovered_by: contributor
scope:
  - crates/cscript-ffi/tests/**
  - crates/general-simulator-cli/tests/**
severity: low
status: fixed
reproducibility: intermittent
tags: [testing, parallelism, cscript-ffi, general-simulator-cli]
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

`crates/cscript-ffi/tests/fixtures.rs`'s `compile_fixture` wrote every compiled `.so` to one
shared, non-unique path: `std::env::temp_dir().join("cscript-ffi-test-fixtures")`, i.e.
`$TMPDIR/cscript-ffi-test-fixtures/lib<name>.so`. Rust's test harness runs tests in
parallel by default; two tests that both use the same fixture name (e.g.
`stateful_fixture_accumulates_across_calls` and
`two_instances_of_the_same_library_keep_independent_state`, both using `libaccumulator.so`)
race: one thread's `cc -shared -o libaccumulator.so ...` (which **truncates the file in place**
before writing) can be mid-write exactly when another thread's `dlopen` reads the same path,
producing a nondeterministic `dlopen failed`.

Three things compounded it:

1. Every test recompiled its fixture, so the window was open once per test, not once per
   binary.
2. The output was written straight to the final path, so a reader could observe a truncated
   file.
3. The directory was shared across *processes* too: `general-simulator-cli`'s
   `tests/cscript.rs`, `tests/sample_time_cli.rs` and `tests/cscript_update_cli.rs` each had
   their own copy of the same helper writing into
   `general-simulator-cli-test-fixtures/lib<name>.so`, and a second checkout or worktree
   running `cargo test` at the same time (another agent's worktree sharing the same `TMPDIR`
   was doing exactly that while this was fixed) rewrites the same file under a running test's
   spawned CLI child.

Confirmed on `d5a8fa3` before the fix: 8 consecutive `cargo test -p cscript-ffi` runs, 1 failed
(3 of 15 tests, all `libaccumulator.so` users).

## Fix or workaround

Fixed on `fix/cscript-fixture-race` (#6). Both compile helpers
(`crates/cscript-ffi/tests/fixtures.rs::compile_source` and
`crates/general-simulator-cli/tests/support/mod.rs::compile_c_fixture`, the latter shared by
the three CLI test binaries) now follow one contract:

- **Compile once per test process.** A `static OnceLock<Mutex<HashMap<String, PathBuf>>>`
  registry maps fixture name to compiled path; the mutex is held across the `cc` call, so two
  threads asking for the same fixture never compile it twice, and the second just gets the path.
- **Content-hashed output directory.** The `.so` lives in
  `<temp>/<crate>-test-fixtures/<hash of source + cc flags>/lib<name>.so`, so an edited
  fixture never reuses a stale library and an unchanged one built by an earlier process is
  safely reused.
- **Never overwrite in place.** `cc` writes to `lib<name>.so.<pid>.tmp` and the result is
  `rename`d over the final path — atomic on POSIX, and a library another thread or process
  already `dlopen`ed keeps its old inode mapped, so it never sees a half-written file.

`cargo test --workspace -- --test-threads=1` remains a valid (slow) workaround for anyone on a
branch predating the fix.

## Prevention

- Any new test that compiles a C fixture must go through those helpers, not its own
  `Command::new("cc")` — grep `Command::new("cc")` under `crates/*/tests` before adding one.
- Re-run the crate's own suite a handful of times at default parallelism before merging any
  change to a compile helper: `for i in 1 2 3 4 5; do cargo test -p cscript-ffi || break; done`.

## References

- Issue #6 (`cscript-ffi tests fail intermittently under the default parallel test runner`).
- Found while verifying an unrelated SPICE raw-file-output feature (`general-simulator-cli`);
  confirmed via 4 repeated runs that the failure is present on code that feature never touches.

## History

- 2026-08-29: Discovered and root-caused during a routine pre-commit verification pass.
- 2026-09-11: Filed as #6 while landing `--out-every` (#5).
- 2026-09-22: Fixed — once-per-process compile behind a mutex, content-hashed output directory,
  temp-file-plus-rename; 6 consecutive `cargo test --workspace --features python` runs and 10
  consecutive `cargo test -p cscript-ffi` runs, 0 failures.
