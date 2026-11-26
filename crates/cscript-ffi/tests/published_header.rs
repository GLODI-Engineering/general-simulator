//! The published `include/gsim_cscript.h` is the ABI, proved by compiling against it.
//!
//! A header that merely *looks* like the contract is worth less than no header at all: it moves
//! the hand-typed prototype from the user's file into this repository and stops anyone
//! questioning it. So these tests do not compare text. They compile a block that includes the
//! header and defines nothing else -- so every prototype comes from the header -- and then load
//! and call it through the real `CScriptRegistry`.
//!
//! That is the strongest statement available on one platform. A C compiler will not let a
//! definition disagree with a prior declaration in the same translation unit, so the block's
//! `cscript_output` is the header's `cscript_output`; and the loader calling it correctly means
//! the header's `cscript_output` is the engine's. The two halves together pin the header to the
//! ABI without any string matching.
//!
//! Why it matters more than the usual test of this kind: a wrong `cscript_output` declaration
//! is invisible on x86-64 Linux. Integer/pointer and floating-point arguments come from separate
//! register files there, so the plausible wrong guess
//! `(state, t, dt, in, in_len, out, out_len)` lines up register for register and produces
//! correct numbers. Windows x64 passes the first four arguments positionally regardless of
//! class, so there the same declaration builds a pointer out of an integer. Nothing a Linux CI
//! can run will catch that -- but a header everyone includes means nobody has to.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::process::Command;

use cscript_ffi::CScriptRegistry;

fn include_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("include")
}

/// Compiles `source` with `include/` on the include path and returns the library.
///
/// Its own helper rather than `fixtures.rs`'s: that one takes no `-I`, and the whole point here
/// is that the block's prototypes come from the header rather than from its own text. The
/// write-to-temp-then-rename discipline and the source-keyed output directory are the same, for
/// the same reason (GOTCHA-001).
fn compile_against_the_header(name: &str, source: &str) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    source.hash(&mut hasher);
    std::fs::read_to_string(include_dir().join("gsim_cscript.h"))
        .expect("read the published header")
        .hash(&mut hasher);
    let out_dir = std::env::temp_dir()
        .join("cscript-ffi-published-header")
        .join(format!("{:016x}", hasher.finish()));
    std::fs::create_dir_all(&out_dir).expect("create output dir");
    let lib_path = out_dir.join(format!("lib{name}.so"));
    if lib_path.exists() {
        return lib_path;
    }

    let source_path = out_dir.join(format!("{name}.{}.c", std::process::id()));
    let tmp_path = out_dir.join(format!("lib{name}.so.{}.tmp", std::process::id()));
    std::fs::write(&source_path, source).expect("write source");
    let output = Command::new("cc")
        .args(["-shared", "-fPIC", "-O0", "-Wall", "-Wextra", "-Werror"])
        .arg("-I")
        .arg(include_dir())
        .arg("-o")
        .arg(&tmp_path)
        .arg(&source_path)
        .output()
        .expect("run cc");
    assert!(
        output.status.success(),
        "a block including the published header must compile with -Wall -Wextra -Werror:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::rename(&tmp_path, &lib_path).expect("move the library into place");
    let _ = std::fs::remove_file(&source_path);
    lib_path
}

/// The plain contract, with every prototype taken from the header.
///
/// `-Werror` matters here: a definition that disagreed with the header's declaration would be a
/// conflicting-types error, and a definition the header did not declare would be a
/// missing-prototype warning rather than a silent pass.
#[test]
fn a_block_whose_prototypes_come_from_the_header_loads_and_computes() {
    let lib = compile_against_the_header(
        "header_gain",
        r#"
#include "gsim_cscript.h"

int cscript_arity(int *in_len, int *out_len) {
    *in_len = 1;
    *out_len = 1;
    return 0;
}

void *cscript_start(void) { return NULL; }

void cscript_output(void *state, const double *in, int in_len, double dt, double *out,
                    int out_len) {
    (void)state;
    (void)in_len;
    (void)dt;
    (void)out_len;
    out[0] = 2.0 * in[0];
}
"#,
    );

    let mut registry = CScriptRegistry::new();
    let mut instance = registry
        .instantiate(&lib)
        .expect("a block built against the published header must load");
    assert_eq!(
        instance.declared_arity(),
        Some(cscript_ffi::DeclaredArity {
            inputs: Some(1),
            outputs: Some(1)
        }),
        "the header's `cscript_arity` is not the engine's"
    );
    assert_eq!(instance.call(&[21.0], 1e-5, 1), vec![42.0]);
    assert_eq!(instance.call(&[1.5], 1e-5, 1), vec![3.0]);
}

/// `dt` arrives as `dt` and `in` arrives as `in`.
///
/// The first test would pass even if `dt` were mangled, because it ignores `dt`. This one
/// returns `dt` itself and the input beside it, so a declaration that crossed the two would
/// come back with the wrong numbers in both columns. On x86-64 it cannot cross them; on Windows
/// x64 a wrong declaration would, and this is the assertion that would then fire.
#[test]
fn the_header_puts_dt_and_the_input_where_the_engine_puts_them() {
    let lib = compile_against_the_header(
        "header_echo",
        r#"
#include "gsim_cscript.h"

int cscript_arity(int *in_len, int *out_len) {
    *in_len = 1;
    *out_len = 2;
    return 0;
}

void *cscript_start(void) { return NULL; }

void cscript_output(void *state, const double *in, int in_len, double dt, double *out,
                    int out_len) {
    (void)state;
    (void)out_len;
    out[0] = dt;
    /* `in_len` as a value, not as a pointer: a declaration that confused the two would make
       this a wild read rather than a 1. */
    out[1] = in[0] + (double)in_len;
}
"#,
    );

    let mut registry = CScriptRegistry::new();
    let mut instance = registry.instantiate(&lib).expect("load the echo block");
    let out = instance.call(&[7.0], 2.5e-6, 2);
    assert_eq!(out[0], 2.5e-6, "`dt` did not arrive as `dt`");
    assert_eq!(out[1], 8.0, "`in[0]` or `in_len` did not arrive intact");
}

/// The `xc` contract's two functions, and the optional `cscript_update`, declared only by the
/// header. The point is coverage of the header rather than of the contract, which
/// `fixtures.rs` already exercises: every prototype it publishes should be one a block can
/// actually define.
#[test]
fn the_xc_contract_and_the_optional_functions_are_declared_correctly_too() {
    let lib = compile_against_the_header(
        "header_decay",
        r#"
#include "gsim_cscript.h"
#include <stdlib.h>

void *cscript_start(void) {
    double *tau = malloc(sizeof(double));
    *tau = 1e-3;
    return tau;
}

void cscript_free(void *state) { free(state); }

void *cscript_clone(void *state) {
    double *copy = malloc(sizeof(double));
    *copy = *(const double *)state;
    return copy;
}

void cscript_derivative(void *state, const double *in, int in_len, const double *xc, int xc_len,
                        double *xc_dot, int xc_dot_len) {
    (void)in_len;
    (void)xc_len;
    (void)xc_dot_len;
    xc_dot[0] = (in[0] - xc[0]) / *(const double *)state;
}

void cscript_output_xc(void *state, const double *in, int in_len, double dt, const double *xc,
                       int xc_len, double *out, int out_len) {
    (void)state;
    (void)in;
    (void)in_len;
    (void)dt;
    (void)xc_len;
    (void)out_len;
    out[0] = xc[0];
}

void cscript_update(void *state, const double *in, int in_len, double dt, const double *xc,
                    int xc_len) {
    (void)state;
    (void)in;
    (void)in_len;
    (void)dt;
    (void)xc;
    (void)xc_len;
}

double cscript_next_sample_hit(void *state, const double *in, int in_len, const double *xc,
                               int xc_len) {
    (void)state;
    (void)in;
    (void)in_len;
    (void)xc;
    (void)xc_len;
    return 1e-5;
}
"#,
    );

    let mut registry = CScriptRegistry::new();
    let instance = registry
        .instantiate_xc(&lib)
        .expect("a block built against the header's xc prototypes must load");
    assert!(
        instance.supports_next_sample_hit(),
        "the header's `cscript_next_sample_hit` is not the symbol the engine resolves"
    );
}

/// The header's bytes are reachable from Rust, so a front end can write it beside a scaffolded
/// source without keeping a second copy of the ABI.
#[test]
fn the_header_is_published_to_rust_callers_too() {
    let text = cscript_ffi::header();
    assert!(
        text.contains(
            "void cscript_output(void *state, const double *in, int in_len, double dt, double *out,"
        ),
        "`cscript_ffi::header()` is not the published header"
    );
    assert_eq!(
        text,
        std::fs::read_to_string(include_dir().join("gsim_cscript.h")).unwrap(),
        "`header()` and the file on disk have drifted"
    );
}
