//! Shared helper for CLI integration tests that compile a `tests/fixtures/*.c` block into a
//! shared library with the system `cc` (exactly the build step a real user runs themselves).
//!
//! Concurrency contract (GOTCHA-001): the test harness runs every `#[test]` in a binary on its
//! own thread, and several tests ask for the same fixture while a spawned `general-simulator`
//! child is `dlopen`ing it. Each fixture is therefore compiled at most once per test process
//! (a process-wide registry behind a `Mutex`), into a directory named by a hash of the source
//! and the compile flags, and the `.so` is written under a temporary name and `rename`d into
//! place -- so nobody, in this process or another, ever observes a truncated or half-written
//! library. Never `cc -o <final path>` directly: that truncates the file in place.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

const CC_FLAGS: [&str; 4] = ["-shared", "-fPIC", "-O0", "-o"];

/// Fixture name -> compiled library path, for every fixture this process has already built.
fn compiled() -> &'static Mutex<HashMap<String, PathBuf>> {
    static COMPILED: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();
    COMPILED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Compile `tests/fixtures/<name>.c` into `lib<name>.so` exactly once per process and return
/// its path.
///
/// The output directory is keyed by a hash of the source text and the compiler flags, so a
/// fixture edited between runs never reuses a stale library, and a library left behind by an
/// earlier process is only ever reused when it was built from byte-identical input.
pub fn compile_c_fixture(name: &str) -> PathBuf {
    let mut registry = compiled().lock().expect("fixture registry poisoned");
    if let Some(path) = registry.get(name) {
        return path.clone();
    }

    let source_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.c"));
    let source = std::fs::read_to_string(&source_path)
        .unwrap_or_else(|e| panic!("read fixture source {}: {e}", source_path.display()));

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    CC_FLAGS.hash(&mut hasher);
    source.hash(&mut hasher);
    let out_dir = std::env::temp_dir()
        .join("general-simulator-cli-test-fixtures")
        .join(format!("{:016x}", hasher.finish()));
    std::fs::create_dir_all(&out_dir).expect("create fixture output dir");
    let lib_path = out_dir.join(format!("lib{name}.so"));

    if !lib_path.exists() {
        // Unique per process so two concurrent `cargo test` invocations (e.g. two checkouts
        // sharing one TMPDIR) never write the same temporary file either.
        let tmp_path = out_dir.join(format!("lib{name}.so.{}.tmp", std::process::id()));
        let status = Command::new("cc")
            .args(CC_FLAGS)
            .arg(&tmp_path)
            .arg(&source_path)
            .status()
            .expect("run cc to compile fixture");
        assert!(status.success(), "cc failed to compile fixture {name}");
        // Atomic on POSIX: a reader either sees the old complete inode or the new one.
        std::fs::rename(&tmp_path, &lib_path).expect("move compiled fixture into place");
    }

    registry.insert(name.to_owned(), lib_path.clone());
    lib_path
}
