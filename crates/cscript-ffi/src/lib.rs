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
//! // needed to use this block under general-simulator's adaptive step-size control, which retries a
//! // rejected trial step from an independent copy of every block's state -- see "Adaptive
//! // step-size control and instance cloning" below.
//! void *cscript_clone(void *state);
//! ```
//!
//! ## The optional continuous-state (`xc`) contract
//!
//! A block declaring `xc_count > 0` in its netlist line owns a continuous-state vector the
//! *solver itself* numerically integrates (RK4, one independent integration per block, the
//! same convention [`continuous_blocks::StateSpace::rk4_step`] uses for every other dynamic
//! block in this workspace) -- as opposed to the plain `void *state` blob above, which is
//! never touched by anything but the block's own C code. This is a pure vector-field
//! evaluation the solver calls, not something the block's own `cscript_output` hand-integrates
//! itself (a block that wants to manage its own state by hand, e.g. a hand-rolled `dv/dt` Euler
//! estimate, remains exactly as correct as before this contract existed -- this new contract is
//! specifically for state the solver should own instead).
//!
//! Such a block exports **two more** functions *instead of* `cscript_output` (never both --
//! [`CScriptRegistry::instantiate_xc`] requires exactly these two, [`CScriptRegistry::instantiate`]
//! requires plain `cscript_output`; a library is meant to implement one contract or the other):
//!
//! ```c
//! // Pure vector-field evaluation: dxc/dt = f(t, u, xc, xd). Must not
//! // mutate `state`, `in`, or `xc` -- called up to four times per accepted step (RK4's own
//! // k1..k4 stages), including from now-discarded trial steps under adaptive stepping (see
//! // "Adaptive step-size control and instance cloning" below -- the same "a rejected trial's
//! // mutation never survives" guarantee applies here, so this restriction is about correctness
//! // of a single RK4 step's own math, not about adaptive-step safety).
//! void cscript_derivative(void *state, const double *in, int in_len,
//!                         const double *xc, int xc_len, double *xc_dot, int xc_dot_len);
//!
//! // Used instead of cscript_output. Identical role, with read access to this step's own
//! // already-RK4-integrated xc (the *new*, post-step continuous state -- matching every other
//! // dynamic block's own "step state, then compute output from the new state" convention). May
//! // still mutate `state` for its own xd/bookkeeping purposes, exactly like plain
//! // cscript_output.
//! void cscript_output_xc(void *state, const double *in, int in_len, double dt,
//!                        const double *xc, int xc_len, double *out, int out_len);
//! ```
//!
//! `xc` itself lives in `dae-runtime`'s own `BlockState`, not inside the opaque `void *state`
//! blob -- the one place this contract breaks the "everything is opaque to Rust" rule, and
//! deliberately so: the solver's own RK4 stepper needs to do ordinary vector arithmetic on it
//! between stages, which an opaque C-managed blob could never allow. `cscript_start`/
//! `cscript_free`/`cscript_clone` are unaffected either way -- `xc`'s own initial value is
//! always the zero vector (matching every other dynamic block's own "starts at rest"
//! convention), and cloning it is a plain `Vec<f64>` clone, needing no C-side involvement at
//! all.
//!
//! ## The optional discrete-state update function
//!
//! `cscript_output`/`cscript_output_xc` are permitted to mutate `state` themselves (documented
//! above) -- fine for the common case, but it conflates two different things: computing this
//! step's output from the block's current state, and committing that state forward to the next
//! step. Some block-diagram tools' own code-block feature keeps these deliberately separate (an
//! output function that's expected to be side-effect-free, plus a dedicated update function
//! that's the only place discrete state actually advances) -- `cscript` now offers the same
//! split, as an **entirely optional** third function:
//!
//! ```c
//! // Optional. Called once per resolved (or per-sample-period, under ts=) step, immediately
//! // after cscript_output/cscript_output_xc for that same call -- the dedicated place to
//! // commit `state`'s own discrete bookkeeping forward to the next step, instead of doing it
//! // inside the output function itself. `xc` is this step's own already-integrated continuous
//! // state (`xc_len == 0` for a block with `xc_count == 0`, exactly like every other xc-aware
//! // parameter in this contract). If this symbol is absent, a cscript block is expected to
//! // keep updating `state` directly inside cscript_output/cscript_output_xc, exactly as before
//! // this function existed -- nothing about the existing contract changes if you never add
//! // this.
//! void cscript_update(void *state, const double *in, int in_len, double dt,
//!                      const double *xc, int xc_len);
//! ```
//!
//! ## Adaptive step-size control and instance cloning
//!
//! general-simulator's adaptive step-size control retries a rejected trial step from scratch: every
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
type DerivativeFn = unsafe extern "C" fn(
    state: *mut c_void,
    input: *const f64,
    in_len: i32,
    xc: *const f64,
    xc_len: i32,
    xc_dot: *mut f64,
    xc_dot_len: i32,
);
type OutputXcFn = unsafe extern "C" fn(
    state: *mut c_void,
    input: *const f64,
    in_len: i32,
    dt: f64,
    xc: *const f64,
    xc_len: i32,
    output: *mut f64,
    out_len: i32,
);
type UpdateFn = unsafe extern "C" fn(
    state: *mut c_void,
    input: *const f64,
    in_len: i32,
    dt: f64,
    xc: *const f64,
    xc_len: i32,
);

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
    // `Option`, not required at load time: which of `output` or (`derivative` + `output_xc`)
    // is actually required depends on how this library is instantiated (plain
    // `CScriptRegistry::instantiate` vs. `instantiate_xc`) — see each one's own doc comment.
    // `load` resolves whatever is present; the two `instantiate*` methods enforce the specific
    // combination they each need.
    output: Option<OutputFn>,
    derivative: Option<DerivativeFn>,
    output_xc: Option<OutputXcFn>,
    // Always optional regardless of contract (plain or xc) -- see the module doc comment,
    // "The optional discrete-state update function." Never required by either `instantiate*`
    // method.
    update: Option<UpdateFn>,
    free: Option<FreeFn>,
    clone_state: Option<CloneFn>,
}

impl CScriptLibrary {
    /// Loads `path` and resolves `cscript_start` (always required) plus every optional symbol
    /// this crate knows about (`cscript_output`, `cscript_derivative`, `cscript_output_xc`,
    /// `cscript_update`, `cscript_free`, `cscript_clone`) — see the module doc comment for what
    /// each one's absence means. Which combination is actually *required* for a given
    /// instantiation is checked separately, by
    /// [`CScriptRegistry::instantiate`]/[`CScriptRegistry::instantiate_xc`].
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
        let output: Option<OutputFn> = unsafe {
            library
                .get(b"cscript_output\0")
                .ok()
                .map(|symbol: Symbol<OutputFn>| *symbol)
        };
        let derivative: Option<DerivativeFn> = unsafe {
            library
                .get(b"cscript_derivative\0")
                .ok()
                .map(|symbol: Symbol<DerivativeFn>| *symbol)
        };
        let output_xc: Option<OutputXcFn> = unsafe {
            library
                .get(b"cscript_output_xc\0")
                .ok()
                .map(|symbol: Symbol<OutputXcFn>| *symbol)
        };
        let update: Option<UpdateFn> = unsafe {
            library
                .get(b"cscript_update\0")
                .ok()
                .map(|symbol: Symbol<UpdateFn>| *symbol)
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
            derivative,
            output_xc,
            update,
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
    ///
    /// # Panics
    /// If this instance's library doesn't export `cscript_output` — unreachable in practice,
    /// since [`CScriptRegistry::instantiate`] (the only way to obtain an instance meant to use
    /// this method) already requires it up front, the same "checked once, up front" discipline
    /// [`Clone`]'s own `cscript_clone` requirement uses.
    pub fn call(&mut self, input: &[f64], dt: f64, out_len: usize) -> Vec<f64> {
        let output_fn = self.library.output.unwrap_or_else(|| {
            panic!(
                "call requires a library loaded via CScriptRegistry::instantiate (which \
                 already requires cscript_output up front — this should be unreachable)"
            )
        });
        let mut output = vec![0.0_f64; out_len];
        // SAFETY: `input`/`output` are valid slices of the declared lengths for the duration
        // of this call; `in_len`/`out_len` are passed through unchanged so the callee can
        // bounds-check itself. Whether the callee actually respects them is the documented,
        // unenforceable part of the C-side contract (see module doc comment).
        unsafe {
            output_fn(
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

    /// Calls `cscript_output_xc` with this instance's own state, `input`/`dt` as given, `xc` as
    /// this step's own already-integrated continuous-state vector, and an `out_len`-length
    /// zero-initialized output buffer, returning it filled in — the `xc`-owning counterpart to
    /// [`Self::call`]. See the module doc comment, "The optional continuous-state (`xc`)
    /// contract."
    ///
    /// # Panics
    /// If this instance's library doesn't export `cscript_output_xc` — unreachable in
    /// practice, since [`CScriptRegistry::instantiate_xc`] already requires it up front.
    pub fn call_xc(&mut self, input: &[f64], dt: f64, xc: &[f64], out_len: usize) -> Vec<f64> {
        let output_xc = self.library.output_xc.unwrap_or_else(|| {
            panic!(
                "call_xc requires a library loaded via CScriptRegistry::instantiate_xc (which \
                 already requires cscript_output_xc up front — this should be unreachable)"
            )
        });
        let mut output = vec![0.0_f64; out_len];
        // SAFETY: same reasoning as `call`'s own SAFETY comment, extended to `xc`.
        unsafe {
            output_xc(
                self.state,
                input.as_ptr(),
                input.len() as i32,
                dt,
                xc.as_ptr(),
                xc.len() as i32,
                output.as_mut_ptr(),
                out_len as i32,
            );
        }
        output
    }

    /// Calls `cscript_update` with this instance's own state, `input`/`dt`/`xc` as given -- a
    /// no-op if this instance's library didn't export it (see the module doc comment, "The
    /// optional discrete-state update function"). Unlike [`Self::call`]/[`Self::call_xc`], this
    /// is genuinely optional for *both* contracts, so there's no panic path: call it every
    /// step it's due exactly like output, and it simply does nothing if the author never wrote
    /// one.
    pub fn update(&mut self, input: &[f64], dt: f64, xc: &[f64]) {
        let Some(update_fn) = self.library.update else {
            return;
        };
        // SAFETY: same reasoning as `call`'s own SAFETY comment, extended to `xc`.
        unsafe {
            update_fn(
                self.state,
                input.as_ptr(),
                input.len() as i32,
                dt,
                xc.as_ptr(),
                xc.len() as i32,
            );
        }
    }

    /// Evaluates `cscript_derivative` once: given this step's already-resolved inputs and a
    /// candidate continuous-state vector `xc`, returns `dxc/dt`. Takes `&self` rather than
    /// `&mut self` — the C-side contract documents (but, like every other rule in this crate,
    /// cannot enforce) that this call must not mutate `state`, so the Rust type keeps that
    /// intent honest even though the raw pointer underneath is technically mutable.
    ///
    /// # Panics
    /// If this instance's library doesn't export `cscript_derivative` — unreachable in
    /// practice, since [`CScriptRegistry::instantiate_xc`] already requires it up front.
    pub fn derivative(&self, input: &[f64], xc: &[f64]) -> Vec<f64> {
        let derivative_fn = self.library.derivative.unwrap_or_else(|| {
            panic!(
                "derivative requires a library loaded via CScriptRegistry::instantiate_xc \
                 (which already requires cscript_derivative up front — this should be \
                 unreachable)"
            )
        });
        let mut xc_dot = vec![0.0_f64; xc.len()];
        // SAFETY: same reasoning as `call`'s own SAFETY comment, extended to `xc`/`xc_dot`.
        unsafe {
            derivative_fn(
                self.state,
                input.as_ptr(),
                input.len() as i32,
                xc.as_ptr(),
                xc.len() as i32,
                xc_dot.as_mut_ptr(),
                xc_dot.len() as i32,
            );
        }
        xc_dot
    }

    /// RK4-integrates `xc` forward by `dt`, holding `input` fixed across all four stages — the
    /// same zero-order-hold convention `continuous_blocks::StateSpace::rk4_step` uses for every
    /// other dynamic block in this workspace, so a `cscript` block's own continuous state
    /// integrates exactly like a `Pid`/`StateSpace`/`Pmsm`'s does, not by some separate rule.
    /// Calls [`Self::derivative`] up to four times (`k1..k4`) — see that method's own `#Panics`.
    pub fn rk4_step_xc(&self, xc: &[f64], input: &[f64], dt: f64) -> Vec<f64> {
        let add = |a: &[f64], b: &[f64], scale: f64| -> Vec<f64> {
            a.iter().zip(b).map(|(ai, bi)| ai + scale * bi).collect()
        };
        let k1 = self.derivative(input, xc);
        let k2 = self.derivative(input, &add(xc, &k1, dt / 2.0));
        let k3 = self.derivative(input, &add(xc, &k2, dt / 2.0));
        let k4 = self.derivative(input, &add(xc, &k3, dt));
        (0..xc.len())
            .map(|i| xc[i] + (dt / 6.0) * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]))
            .collect()
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
                 (required for general-simulator's adaptive step-size control); either export \
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

    /// Loads `path` if not already cached (resolving every optional symbol this crate knows
    /// about, regardless of which contract the caller ultimately needs — see
    /// [`CScriptLibrary::load`]'s own doc comment), returning the cached/loaded `Arc`.
    fn load_cached(&mut self, path: &Path) -> Result<Arc<CScriptLibrary>, CScriptError> {
        if let Some(library) = self.libraries.get(path) {
            Ok(library.clone())
        } else {
            let library = Arc::new(CScriptLibrary::load(path)?);
            self.libraries.insert(path.to_path_buf(), library.clone());
            Ok(library)
        }
    }

    /// Loads `path` if not already cached, then creates and returns a fresh instance from it
    /// (its own `cscript_start()` call, its own state) using the **plain** contract: requires
    /// `cscript_output` (via [`CScriptInstance::call`]), for a block with `xc_count == 0`.
    pub fn instantiate(&mut self, path: &Path) -> Result<CScriptInstance, CScriptError> {
        let library = self.load_cached(path)?;
        if library.output.is_none() {
            return Err(CScriptError::MissingSymbol {
                path: path.to_path_buf(),
                symbol: "cscript_output",
                message: "not found (or this library only implements the xc-owning contract \
                          — see CScriptRegistry::instantiate_xc)"
                    .to_string(),
            });
        }
        Ok(CScriptInstance::new(library))
    }

    /// Loads `path` if not already cached, then creates and returns a fresh instance from it
    /// using the **continuous-state (`xc`)** contract: requires `cscript_derivative` and
    /// `cscript_output_xc` (via [`CScriptInstance::rk4_step_xc`]/[`CScriptInstance::call_xc`]),
    /// for a block with `xc_count > 0`. See the module doc comment, "The optional
    /// continuous-state (`xc`) contract."
    pub fn instantiate_xc(&mut self, path: &Path) -> Result<CScriptInstance, CScriptError> {
        let library = self.load_cached(path)?;
        if library.derivative.is_none() {
            return Err(CScriptError::MissingSymbol {
                path: path.to_path_buf(),
                symbol: "cscript_derivative",
                message: "required because this block declares xc_count > 0".to_string(),
            });
        }
        if library.output_xc.is_none() {
            return Err(CScriptError::MissingSymbol {
                path: path.to_path_buf(),
                symbol: "cscript_output_xc",
                message: "required because this block declares xc_count > 0".to_string(),
            });
        }
        Ok(CScriptInstance::new(library))
    }
}
