//! Loads a user-supplied, precompiled shared library (`.so`/`.dylib`/`.dll`) exposing three C
//! functions and calls into it once per block instance per transient step — a dynamically-
//! loaded, stateful escape hatch for block behavior no existing `continuous-blocks` block
//! covers, the same role a code-generation/scripting slot plays in other block-diagram
//! simulation tools' own extensibility mechanisms, or an `ngspice` XSPICE codemodel `.cm`
//! plugin (see this workspace's own `internal-archive/gotchas/
//! ngspice-xspice-codemodel-needs-mfbinit-env.md` for that already-used pattern).
//!
//! **This crate is the one place in this workspace where calling into arbitrary native code is
//! deliberately allowed.** Loading a shared library and calling into it is unsafe by
//! construction: the code runs in-process with this program's own privileges, can corrupt
//! memory, crash the process, or do anything a normal native program can do. That is an
//! accepted, bounded risk of this specific opt-in feature (a user explicitly names a `.so` file
//! in their own netlist), not something this crate tries to sandbox away — no different in kind
//! from the XSPICE codemodel pattern already used elsewhere in this same project.
//!
//! ## The C-side contract
//!
//! A block's shared library must export:
//!
//! ```c
//! // Called once, when this block instance is created. Return an opaque instance-state
//! // pointer (typically malloc'd), or NULL if this block instance carries no state at all.
//! void *cscript_start(void);
//!
//! // Called once per resolved transient step. `state` is whatever cscript_start() returned
//! // for this instance (NULL if none); `in`/`in_len` is this step's input vector, in the
//! // netlist's own declared order; `dt` is this step's size (seconds) -- needed for any block
//! // that integrates over time, e.g. `state->integral += error * dt`, the same explicit-Euler
//! // update every other stateful block in this project already uses; `out`/`out_len` is the
//! // output vector to fill.
//! void cscript_output(void *state, const double *in, int in_len, double dt, double *out, int out_len);
//!
//! // Optional. Called once when this block instance is dropped. Free anything
//! // cscript_start() allocated. If the symbol is absent, `state` is simply leaked -- harmless
//! // for a short-lived CLI run, but real cleanup is provided for any longer-lived host.
//! void cscript_free(void *state);
//!
//! // Optional. Return a deep copy of `state` (independent memory: mutating the copy through
//! // future cscript_output() calls must never affect the original, and vice versa). Only
//! // needed to use this block under elspice-pwl's adaptive step-size control, which retries a
//! // rejected trial step from an independent copy of every block's state -- see "Adaptive
//! // step-size control and instance cloning" below.
//! void *cscript_clone(void *state);
//! ```
//!
//! ## Adaptive step-size control and instance cloning
//!
//! elspice-pwl's adaptive step-size control retries a rejected trial step from scratch: every
//! block's state is cloned before the trial, the trial runs against the clone, and if rejected
//! the clone is discarded and a fresh one is taken for the next, smaller attempt -- never
//! mutating the real, confirmed state until a trial is actually accepted. That is exactly
//! correct for plain Rust block state (a `Vec<f64>` clone is a real, independent copy), but a
//! [`CScriptInstance`]'s state is an opaque pointer this crate never reads through, so it cannot
//! generically deep-copy it -- only the user's own C code knows its own layout. [`Clone`] for
//! [`CScriptInstance`] is implemented in terms of `cscript_clone` for exactly this reason, and
//! **panics** if the library didn't export it. In practice this should never fire: check
//! [`CScriptInstance::supports_clone`] once, up front, and refuse adaptive step-size control
//! for a netlist containing a `cscript` block whose library doesn't support it, the same way
//! `dae-runtime` already does before entering its own adaptive loop.
//!
//! Everything else in the user's `.c` file — includes, `#define`s, `typedef`s, file-scope
//! `static` helper functions and variables the user wants visible to all three of the functions
//! above for one translation unit — needs no special support from this crate: plain C file
//! scope already provides exactly that. The user compiles their own `.c` file into a shared
//! library themselves (e.g. `cc -shared -fPIC -o my_block.so my_block.c`); this crate never
//! invokes a compiler.
//!
//! `in_len`/`out_len` must match the netlist's own declared input/output counts for this block
//! — nothing in this crate checks that they do; a mismatch is a plain out-of-bounds C array
//! access, same as it would be calling `OutputSignal(9, 0)` past a declared output count in any
//! other tool's own code-slot mechanism. This is documented, not enforced.
//!
//! ## Multi-instance state
//!
//! Two block instances naming the *same* `lib=` path share one loaded image (`dlopen` on most
//! platforms de-duplicates repeat loads of the same path — see [`CScriptLibrary::load`]), but
//! each instance gets its *own* `cscript_start()` call and its own returned state pointer, so
//! per-instance state (e.g. an integrator's accumulator) is never accidentally shared between
//! two uses of the same block in one netlist. [`CScriptRegistry`] caches loaded libraries by
//! path for exactly this reason: cheap repeat instantiation, without repeat `dlopen`.

use std::collections::BTreeMap;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use libloading::{Library, Symbol};

type StartFn = unsafe extern "C" fn() -> *mut c_void;
type OutputFn = unsafe extern "C" fn(
    state: *mut c_void,
    input: *const f64,
    in_len: i32,
    dt: f64,
    output: *mut f64,
    out_len: i32,
);
type FreeFn = unsafe extern "C" fn(state: *mut c_void);
type CloneFn = unsafe extern "C" fn(state: *mut c_void) -> *mut c_void;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CScriptError {
    Load {
        path: PathBuf,
        message: String,
    },
    MissingSymbol {
        path: PathBuf,
        symbol: &'static str,
        message: String,
    },
}

impl std::fmt::Display for CScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CScriptError::Load { path, message } => {
                write!(
                    f,
                    "failed to load C-script library {}: {message}",
                    path.display()
                )
            }
            CScriptError::MissingSymbol {
                path,
                symbol,
                message,
            } => write!(
                f,
                "C-script library {} is missing required symbol `{symbol}`: {message}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for CScriptError {}

/// One loaded shared library's three FFI entry points. Kept alive for as long as any
/// [`CScriptInstance`] built from it exists (each instance holds an `Arc` to this) — the
/// `Library` must outlive every raw function pointer taken from it.
pub struct CScriptLibrary {
    // Held only to keep the mapped image alive; never read directly again after `load`.
    _library: Library,
    start: StartFn,
    output: OutputFn,
    free: Option<FreeFn>,
    clone_state: Option<CloneFn>,
}

impl CScriptLibrary {
    /// Loads `path` and resolves `cscript_start`/`cscript_output` (required) and
    /// `cscript_free`/`cscript_clone` (both optional — see the module doc comment for what
    /// each one's absence means).
    pub fn load(path: &Path) -> Result<Self, CScriptError> {
        // SAFETY: dlopen-ing and resolving symbols from a user-specified path is exactly the
        // unsafety this crate exists to contain -- see the module doc comment. The caller
        // chose this path deliberately, in their own netlist.
        let library = unsafe { Library::new(path) }.map_err(|e| CScriptError::Load {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;

        let start: StartFn = unsafe {
            let symbol: Symbol<StartFn> =
                library
                    .get(b"cscript_start\0")
                    .map_err(|e| CScriptError::MissingSymbol {
                        path: path.to_path_buf(),
                        symbol: "cscript_start",
                        message: e.to_string(),
                    })?;
            *symbol
        };
        let output: OutputFn = unsafe {
            let symbol: Symbol<OutputFn> =
                library
                    .get(b"cscript_output\0")
                    .map_err(|e| CScriptError::MissingSymbol {
                        path: path.to_path_buf(),
                        symbol: "cscript_output",
                        message: e.to_string(),
                    })?;
            *symbol
        };
        let free: Option<FreeFn> = unsafe {
            library
                .get(b"cscript_free\0")
                .ok()
                .map(|symbol: Symbol<FreeFn>| *symbol)
        };
        let clone_state: Option<CloneFn> = unsafe {
            library
                .get(b"cscript_clone\0")
                .ok()
                .map(|symbol: Symbol<CloneFn>| *symbol)
        };

        Ok(CScriptLibrary {
            _library: library,
            start,
            output,
            free,
            clone_state,
        })
    }
}

/// One block instance's own state, from one call to `cscript_start()` on a shared
/// [`CScriptLibrary`]. `Drop` calls `cscript_free` if the library exported one.
pub struct CScriptInstance {
    library: Arc<CScriptLibrary>,
    state: *mut c_void,
}

impl std::fmt::Debug for CScriptInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CScriptInstance")
            .field("state", &self.state)
            .finish()
    }
}

// The state pointer is opaque to us and only ever passed back into the same library's own
// functions, which are the sole thing that dereferences it -- this crate never reads through
// it, so moving the pointer between threads is exactly as safe as moving the shared library
// handle itself. Circuit simulation in this workspace is single-threaded, so this is unused in
// practice, but the type would otherwise fail to be `Send` at all without it.
unsafe impl Send for CScriptInstance {}

impl CScriptInstance {
    fn new(library: Arc<CScriptLibrary>) -> Self {
        // SAFETY: calling into user-supplied native code with no arguments, as documented in
        // the module doc comment's C-side contract.
        let state = unsafe { (library.start)() };
        CScriptInstance { library, state }
    }

    /// Calls `cscript_output` with this instance's own state, `input` and `dt` as given, and
    /// an `out_len`-length zero-initialized output buffer, returning it filled in.
    pub fn call(&mut self, input: &[f64], dt: f64, out_len: usize) -> Vec<f64> {
        let mut output = vec![0.0_f64; out_len];
        // SAFETY: `input`/`output` are valid slices of the declared lengths for the duration
        // of this call; `in_len`/`out_len` are passed through unchanged so the callee can
        // bounds-check itself. Whether the callee actually respects them is the documented,
        // unenforceable part of the C-side contract (see module doc comment).
        unsafe {
            (self.library.output)(
                self.state,
                input.as_ptr(),
                input.len() as i32,
                dt,
                output.as_mut_ptr(),
                out_len as i32,
            );
        }
        output
    }

    /// Whether this instance's library exported `cscript_clone` — check this once, up front,
    /// before deciding whether a netlist using this block is eligible for adaptive step-size
    /// control (see the module doc comment, "Adaptive step-size control and instance cloning").
    pub fn supports_clone(&self) -> bool {
        self.library.clone_state.is_some()
    }

    /// A real, independent deep copy of this instance's state via `cscript_clone`, or `None`
    /// if the library didn't export it. Prefer this over [`Clone::clone`] wherever a missing
    /// `cscript_clone` should be handled as an ordinary error rather than a panic.
    pub fn try_clone(&self) -> Option<CScriptInstance> {
        let clone_fn = self.library.clone_state?;
        // SAFETY: `self.state` was returned by this same library's own `cscript_start` (or a
        // prior `cscript_clone`); the callee is expected to return an independent copy, per
        // the C-side contract documented in the module doc comment.
        let state = unsafe { clone_fn(self.state) };
        Some(CScriptInstance {
            library: self.library.clone(),
            state,
        })
    }
}

impl Clone for CScriptInstance {
    /// # Panics
    /// If this instance's library didn't export `cscript_clone`. See the module doc comment,
    /// "Adaptive step-size control and instance cloning" — callers that need to handle this
    /// without panicking should check [`CScriptInstance::supports_clone`] first, or use
    /// [`CScriptInstance::try_clone`] directly.
    fn clone(&self) -> Self {
        self.try_clone().unwrap_or_else(|| {
            panic!(
                "cannot clone a CScriptInstance whose library does not export cscript_clone \
                 (required for elspice-pwl's adaptive step-size control); either export \
                 cscript_clone from this library or run with a fixed --dt"
            )
        })
    }
}

impl Drop for CScriptInstance {
    fn drop(&mut self) {
        if let Some(free) = self.library.free {
            if !self.state.is_null() {
                // SAFETY: `state` was returned by this same library's own `cscript_start`, and
                // this is the only place it is ever freed.
                unsafe { free(self.state) };
            }
        }
    }
}

/// Caches loaded libraries by path, so several block instances naming the same `lib=` file
/// share one `dlopen`'d image while each still gets its own [`CScriptInstance`] (own state
/// pointer) — see the module doc comment, "Multi-instance state."
#[derive(Default)]
pub struct CScriptRegistry {
    libraries: BTreeMap<PathBuf, Arc<CScriptLibrary>>,
}

impl CScriptRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Loads `path` if not already cached, then creates and returns a fresh instance from it
    /// (its own `cscript_start()` call, its own state).
    pub fn instantiate(&mut self, path: &Path) -> Result<CScriptInstance, CScriptError> {
        let library = if let Some(library) = self.libraries.get(path) {
            library.clone()
        } else {
            let library = Arc::new(CScriptLibrary::load(path)?);
            self.libraries.insert(path.to_path_buf(), library.clone());
            library
        };
        Ok(CScriptInstance::new(library))
    }
}
