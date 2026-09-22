//! Embeds a Python interpreter (PyO3's `auto-initialize` feature) and calls user-supplied
//! Python functions once per resolved block-graph step -- the Python-hosted counterpart to
//! `cscript_ffi`'s dynamically-linked C blocks. See `general-simulator`'s own
//! `book/dev-guide/src/python-blocks.md` for the full design and the measurements behind it;
//! this module doc comment covers the Python-side contract only.
//!
//! ## The Python-side contract
//!
//! A block's `.py` file must define:
//!
//! ```python
//! def start():
//!     # Called once, when this block instance is created. Return any Python object (a dict, a
//!     # plain class instance, ...) as this instance's own persistent state, or None. Put every
//!     # import, every precomputed table, every one-time-expensive thing here -- never in
//!     # output(). See the module doc comment's own measurements for why this matters.
//!     ...
//!
//! def output(state, t, dt, inputs):
//!     # Called once per resolved step (or once per sample period under ts=, mirroring
//!     # cscript's own zero-order-hold convention). `inputs` is a list, one entry per declared
//!     # `inputs=` signal -- a float for a scalar signal, a numpy.ndarray for a vector one
//!     # (never flattened into one array the way cscript's C `in[]` has to be -- Python can
//!     # keep each declared input's own shape). Return a single float, or a tuple/list of
//!     # floats matching this block's own declared `outputs=` count (deliberately scalar-only
//!     # outputs -- see the design doc).
//!     ...
//! ```
//!
//! ## The optional continuous-state (`xc`) contract
//!
//! A block declaring `xc_count > 0` exports `derivative`/`output_xc` *instead of* `output` --
//! the exact same split `cscript`'s own `xc` contract uses, for the same reason (a state the
//! *solver* numerically integrates, not the block's own hand-rolled update):
//!
//! ```python
//! def derivative(state, t, inputs, xc):
//!     # xc: numpy.ndarray, this RK4 stage's own candidate continuous-state vector. Must not
//!     # mutate `state` -- called up to four times per accepted step. Return dxc/dt,
//!     # array-like, same length as xc.
//!     ...
//!
//! def output_xc(state, t, dt, inputs, xc):
//!     # xc: numpy.ndarray, this step's own already-integrated continuous state. May still
//!     # mutate `state` for its own discrete bookkeeping, exactly like output() can.
//!     ...
//! ```
//!
//! ## The optional discrete-state update function
//!
//! `output`/`output_xc` are permitted to mutate `state` themselves (documented above) -- fine
//! for the common case, but it conflates computing this step's output with committing state
//! forward to the next step. Some block-diagram tools' own code-block feature keeps these
//! deliberately separate (an output function expected to be side-effect-free, plus a dedicated
//! update function that's the only place discrete state actually advances) -- `pyblock` now
//! offers the same split, as an **entirely optional** function, mirroring `cscript`'s own
//! `cscript_update`:
//!
//! ```python
//! def update(state, t, dt, inputs):
//!     # Optional. Called once per resolved (or per-sample-period, under ts=) step, immediately
//!     # after output(). The dedicated place to commit `state`'s own discrete bookkeeping
//!     # forward to the next step, instead of doing it inside output() itself. If this function
//!     # is absent, a pyblock is expected to keep updating `state` directly inside output(),
//!     # exactly as before this function existed.
//!     ...
//!
//! # For a block declaring xc_count > 0, update() additionally receives this step's own
//! # already-integrated continuous state, mirroring output_xc's own extra parameter:
//! def update(state, t, dt, inputs, xc):
//!     ...
//! ```
//!
//! `update`'s own arity (four or five positional parameters) is inferred from which contract
//! this instance was created under ([`PyBlockRegistry::instantiate`] vs.
//! `instantiate_xc`) -- there is no separate `update_xc` name, the same one `update` function
//! just receives one more argument under the `xc` contract, matching `output`/`output_xc`'s own
//! naming split being about the *return* value (`y`) rather than the input shape here.
//!
//! ## The optional block-controlled sample time
//!
//! A block's own `ts=`/`freq=` fixes a period at netlist-parse time. Some blocks instead know
//! their own *next* execution time only at runtime — `ts=variable` in the netlist selects this
//! mode, mirroring `cscript`'s own `cscript_next_sample_hit`, and **requires** the `.py` file to
//! define one more function (checked once, up front, by whichever caller resolves the netlist's
//! own `ts=variable` request — this crate itself never enforces it, the same way it never
//! enforces which of `output`/`output_xc` a plain vs. `xc`-contract instance needs):
//!
//! ```python
//! def next_sample_hit(state, t, dt, inputs):        # plain contract
//!     # Required when this block declares ts=variable (never called otherwise). Called
//!     # immediately after output(). Returns the number of seconds, relative to this call
//!     # (a duration, not an absolute time stamp — even though `t` itself is absolute, matching
//!     # output's own signature, what this function *returns* is deliberately not), until this
//!     # block should next be executed. Must return a value > 0.
//!     ...
//! def next_sample_hit(state, t, dt, inputs, xc):    # xc contract -- one extra argument
//!     ...
//! ```
//!
//! ## State, cloning, and per-instance isolation
//!
//! Each instance gets its own Python namespace (`PyModule::from_code` execs the cached source
//! text fresh per instance), so two instances of the same `.py` file never share module-level
//! state -- while still sharing the *process-wide* `sys.modules` cache, so `import numpy`
//! genuinely only pays its real cost once per process, not once per instance. The registry
//! caches each file's own *source text* (avoiding repeat disk reads across instances), not a
//! separately-compiled code object -- `from_code` recompiles per instance, which is fast enough
//! (well under a millisecond for a block-sized file) not to be worth a second cache layer.
//!
//! Cloning (needed only for [`PyBlockInstance::try_clone`], itself only needed under adaptive
//! step-size control's own trial-and-discard retry loop) uses Python's own generic
//! `copy.deepcopy` on the instance's state object -- unlike `cscript`, there is no author
//! opt-in required and no "doesn't support clone" rejection path: `deepcopy` works on ordinary
//! Python state automatically. A state object that genuinely can't be deep-copied (an open file
//! handle) surfaces as an ordinary Python exception -> [`PyBlockError::Exception`], not a panic.

use std::ffi::CString;
use std::path::{Path, PathBuf};

use pyo3::prelude::*;
use pyo3::types::PyModule;

#[derive(Debug, Clone, PartialEq)]
pub enum PyBlockError {
    Load {
        path: PathBuf,
        message: String,
    },
    MissingFunction {
        path: PathBuf,
        name: &'static str,
        message: String,
    },
    Exception {
        path: PathBuf,
        function: &'static str,
        message: String,
    },
}

impl std::fmt::Display for PyBlockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PyBlockError::Load { path, message } => {
                write!(
                    f,
                    "failed to load Python block {}: {message}",
                    path.display()
                )
            }
            PyBlockError::MissingFunction {
                path,
                name,
                message,
            } => write!(
                f,
                "Python block {} is missing required function `{name}`: {message}",
                path.display()
            ),
            PyBlockError::Exception {
                path,
                function,
                message,
            } => write!(
                f,
                "Python block {} raised in `{function}`: {message}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for PyBlockError {}

/// One input value passed to a Python block's own `output`/`derivative` function -- a plain
/// scalar or a fixed-length vector, converted to a Python `float`/`numpy.ndarray` respectively.
/// Deliberately not `general_mna::block_graph::SignalValue` itself, to keep this crate free of
/// a dependency on `general-mna` -- matching `cscript-ffi`'s own convention of taking plain
/// `f64` slices and leaving any scalar/vector decision to the caller (`dae-runtime`).
#[derive(Debug, Clone, Copy)]
pub enum PyInput<'a> {
    Scalar(f64),
    Vector(&'a [f64]),
}

fn py_traceback(_py: Python<'_>, err: &PyErr) -> String {
    err.to_string()
}

fn build_args<'py>(
    py: Python<'py>,
    inputs: &[PyInput],
) -> PyResult<pyo3::Bound<'py, pyo3::types::PyList>> {
    use numpy::PyArray1;
    use pyo3::types::PyList;
    let list = PyList::empty(py);
    for input in inputs {
        match input {
            PyInput::Scalar(x) => list.append(*x)?,
            PyInput::Vector(xs) => list.append(PyArray1::from_slice(py, xs))?,
        }
    }
    Ok(list)
}

/// Extracts `output()`/`output_xc()`'s own return value: a bare `float` when the block
/// declares exactly one output (the natural Python idiom -- `return x`, not `return [x]`), or a
/// `tuple`/`list` of `float`s matching a multi-output block's own declared count.
fn extract_outputs(result: pyo3::Bound<'_, PyAny>, out_len: usize) -> PyResult<Vec<f64>> {
    if out_len <= 1 {
        let v: f64 = result.extract()?;
        return Ok(vec![v]);
    }
    extract_vector(result)
}

/// Extracts `derivative()`'s own return value: always array-like (`dxc/dt`, one entry per `xc`
/// element), regardless of `xc`'s own length -- unlike [`extract_outputs`], a length-1
/// continuous state is still a state *vector* conceptually, so `derivative()` never gets the
/// bare-scalar shortcut (its own docstring says "array-like, same length as xc," not "a bare
/// float when there's only one").
fn extract_vector(result: pyo3::Bound<'_, PyAny>) -> PyResult<Vec<f64>> {
    let seq = result.try_iter()?;
    let mut out = Vec::new();
    for item in seq {
        out.push(item?.extract::<f64>()?);
    }
    Ok(out)
}

/// Caches each `.py` file's own source text by path -- a fresh [`PyBlockInstance`] execs it
/// into its own new namespace, so multiple instances of the same file never share module-level
/// state (see the module doc comment, "State, cloning, and per-instance isolation").
#[derive(Default)]
pub struct PyBlockRegistry {
    sources: std::collections::BTreeMap<PathBuf, String>,
}

impl PyBlockRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn load_source(&mut self, path: &Path) -> Result<String, PyBlockError> {
        if let Some(source) = self.sources.get(path) {
            return Ok(source.clone());
        }
        let source = std::fs::read_to_string(path).map_err(|e| PyBlockError::Load {
            path: path.to_path_buf(),
            message: e.to_string(),
        })?;
        self.sources.insert(path.to_path_buf(), source.clone());
        Ok(source)
    }

    /// The plain contract: requires `start`/`output`, for a block with `xc_count == 0`.
    pub fn instantiate(&mut self, path: &Path) -> Result<PyBlockInstance, PyBlockError> {
        let source = self.load_source(path)?;
        PyBlockInstance::new(path, &source, false)
    }

    /// The continuous-state (`xc`) contract: requires `start`/`derivative`/`output_xc`, for a
    /// block with `xc_count > 0`.
    pub fn instantiate_xc(&mut self, path: &Path) -> Result<PyBlockInstance, PyBlockError> {
        let source = self.load_source(path)?;
        PyBlockInstance::new(path, &source, true)
    }
}

/// One block instance: its own Python namespace (a fresh module exec'd from the cached source),
/// its own state object (`start()`'s own return value), and the specific function handles this
/// instance's own contract (plain or `xc`) needs.
#[derive(Debug)]
pub struct PyBlockInstance {
    path: PathBuf,
    source: String,
    state: Py<PyAny>,
    output_fn: Option<Py<PyAny>>,
    derivative_fn: Option<Py<PyAny>>,
    output_xc_fn: Option<Py<PyAny>>,
    // Always optional regardless of contract (plain or xc) -- see the module doc comment, "The
    // optional discrete-state update function." Never required by `new`.
    update_fn: Option<Py<PyAny>>,
    // Optional here too, but *required* when this block's netlist line declares `ts=variable`
    // -- that requirement is checked by `dae-runtime`, not this crate. See the module doc
    // comment, "The optional block-controlled sample time."
    next_sample_hit_fn: Option<Py<PyAny>>,
}

impl PyBlockInstance {
    fn new(path: &Path, source: &str, want_xc: bool) -> Result<Self, PyBlockError> {
        Python::attach(|py| {
            let module = exec_fresh_module(py, path, source).map_err(|e| PyBlockError::Load {
                path: path.to_path_buf(),
                message: py_traceback(py, &e),
            })?;

            let get = |name: &'static str| -> Option<Py<PyAny>> {
                module.getattr(name).ok().map(|f| f.unbind())
            };

            let start_fn = get("start").ok_or_else(|| PyBlockError::MissingFunction {
                path: path.to_path_buf(),
                name: "start",
                message: "not found".to_string(),
            })?;
            let output_fn = get("output");
            let derivative_fn = get("derivative");
            let output_xc_fn = get("output_xc");
            let update_fn = get("update");
            let next_sample_hit_fn = get("next_sample_hit");

            if want_xc {
                if derivative_fn.is_none() {
                    return Err(PyBlockError::MissingFunction {
                        path: path.to_path_buf(),
                        name: "derivative",
                        message: "required because this block declares xc_count > 0".to_string(),
                    });
                }
                if output_xc_fn.is_none() {
                    return Err(PyBlockError::MissingFunction {
                        path: path.to_path_buf(),
                        name: "output_xc",
                        message: "required because this block declares xc_count > 0".to_string(),
                    });
                }
            } else if output_fn.is_none() {
                return Err(PyBlockError::MissingFunction {
                    path: path.to_path_buf(),
                    name: "output",
                    message: "not found (or this block only implements the xc contract -- see \
                              PyBlockRegistry::instantiate_xc)"
                        .to_string(),
                });
            }

            let state = start_fn
                .bind(py)
                .call0()
                .map_err(|e| PyBlockError::Exception {
                    path: path.to_path_buf(),
                    function: "start",
                    message: py_traceback(py, &e),
                })?
                .unbind();

            Ok(PyBlockInstance {
                path: path.to_path_buf(),
                source: source.to_string(),
                state,
                output_fn,
                derivative_fn,
                output_xc_fn,
                update_fn,
                next_sample_hit_fn,
            })
        })
    }

    /// Calls `output(state, t, dt, inputs)`, returning `out_len` values (a single-element
    /// `Vec` for a scalar return, or as many elements as the Python function's own returned
    /// tuple/list has).
    ///
    /// # Panics
    /// If this instance was created via [`PyBlockRegistry::instantiate_xc`] (no `output`
    /// function resolved) -- unreachable in practice, since `dae-runtime` only ever calls this
    /// on an instance created via the plain [`PyBlockRegistry::instantiate`].
    pub fn call(
        &mut self,
        t: f64,
        dt: f64,
        inputs: &[PyInput],
        out_len: usize,
    ) -> Result<Vec<f64>, PyBlockError> {
        if self.output_fn.is_none() {
            panic!(
                "call requires an instance created via PyBlockRegistry::instantiate (which \
                 already requires `output` up front -- this should be unreachable)"
            );
        }
        Python::attach(|py| {
            let output_fn = self.output_fn.as_ref().unwrap().clone_ref(py);
            let args = build_args(py, inputs).map_err(|e| self.exception(py, "output", e))?;
            let result = output_fn
                .bind(py)
                .call1((self.state.bind(py), t, dt, args))
                .map_err(|e| self.exception(py, "output", e))?;
            extract_outputs(result, out_len).map_err(|e| self.exception(py, "output", e))
        })
    }

    /// Calls `output_xc(state, t, dt, inputs, xc)`.
    ///
    /// # Panics
    /// If this instance doesn't have an `output_xc` function resolved -- unreachable in
    /// practice, since [`PyBlockRegistry::instantiate_xc`] already requires it up front.
    pub fn call_xc(
        &mut self,
        t: f64,
        dt: f64,
        inputs: &[PyInput],
        xc: &[f64],
        out_len: usize,
    ) -> Result<Vec<f64>, PyBlockError> {
        if self.output_xc_fn.is_none() {
            panic!(
                "call_xc requires an instance created via PyBlockRegistry::instantiate_xc \
                 (which already requires `output_xc` up front -- this should be unreachable)"
            );
        }
        Python::attach(|py| {
            let output_xc_fn = self.output_xc_fn.as_ref().unwrap().clone_ref(py);
            let args = build_args(py, inputs).map_err(|e| self.exception(py, "output_xc", e))?;
            let xc_arr = numpy::PyArray1::from_slice(py, xc);
            let result = output_xc_fn
                .bind(py)
                .call1((self.state.bind(py), t, dt, args, xc_arr))
                .map_err(|e| self.exception(py, "output_xc", e))?;
            extract_outputs(result, out_len).map_err(|e| self.exception(py, "output_xc", e))
        })
    }

    /// Calls `update(state, t, dt, inputs)` -- a no-op if this instance's `.py` file didn't
    /// define `update` (see the module doc comment, "The optional discrete-state update
    /// function"). Unlike [`Self::call`]/[`Self::call_xc`], genuinely optional for both
    /// contracts, so there's no panic path: call it every step it's due exactly like output,
    /// and it simply does nothing if the author never wrote one. For an instance created via
    /// [`PyBlockRegistry::instantiate_xc`], use [`Self::update_xc`] instead (extra `xc`
    /// argument, matching `output`/`output_xc`'s own split).
    pub fn update(&mut self, t: f64, dt: f64, inputs: &[PyInput]) -> Result<(), PyBlockError> {
        if self.update_fn.is_none() {
            return Ok(());
        }
        Python::attach(|py| {
            let update_fn = self.update_fn.as_ref().unwrap().clone_ref(py);
            let args = build_args(py, inputs).map_err(|e| self.exception(py, "update", e))?;
            update_fn
                .bind(py)
                .call1((self.state.bind(py), t, dt, args))
                .map_err(|e| self.exception(py, "update", e))?;
            Ok(())
        })
    }

    /// The `xc`-aware counterpart to [`Self::update`] -- calls `update(state, t, dt, inputs,
    /// xc)`, a no-op if `update` wasn't defined. `xc` should be this step's own already-
    /// integrated continuous state, the same value passed to [`Self::call_xc`].
    pub fn update_xc(
        &mut self,
        t: f64,
        dt: f64,
        inputs: &[PyInput],
        xc: &[f64],
    ) -> Result<(), PyBlockError> {
        if self.update_fn.is_none() {
            return Ok(());
        }
        Python::attach(|py| {
            let update_fn = self.update_fn.as_ref().unwrap().clone_ref(py);
            let args = build_args(py, inputs).map_err(|e| self.exception(py, "update", e))?;
            let xc_arr = numpy::PyArray1::from_slice(py, xc);
            update_fn
                .bind(py)
                .call1((self.state.bind(py), t, dt, args, xc_arr))
                .map_err(|e| self.exception(py, "update", e))?;
            Ok(())
        })
    }

    /// Whether this instance's `.py` file defined `next_sample_hit` — check this once, up
    /// front, when this block's netlist line declares `ts=variable` (see the module doc
    /// comment, "The optional block-controlled sample time") and reject the netlist with a
    /// clear error if not, before ever calling [`Self::next_sample_hit`]/[`Self::next_sample_hit_xc`].
    pub fn supports_next_sample_hit(&self) -> bool {
        self.next_sample_hit_fn.is_some()
    }

    /// Calls `next_sample_hit(state, t, dt, inputs)`, returning the number of seconds
    /// (relative to this call) until this block should next be executed.
    ///
    /// # Panics
    /// If this instance's `.py` file doesn't define `next_sample_hit` — unreachable in
    /// practice, since a caller is expected to check [`Self::supports_next_sample_hit`] once,
    /// up front, before ever using `ts=variable` for this block at all.
    pub fn next_sample_hit(
        &self,
        t: f64,
        dt: f64,
        inputs: &[PyInput],
    ) -> Result<f64, PyBlockError> {
        if self.next_sample_hit_fn.is_none() {
            panic!(
                "next_sample_hit requires a .py file defining next_sample_hit (required for \
                 ts=variable — this should be unreachable, since that's checked up front)"
            );
        }
        Python::attach(|py| {
            let f = self.next_sample_hit_fn.as_ref().unwrap().clone_ref(py);
            let args =
                build_args(py, inputs).map_err(|e| self.exception(py, "next_sample_hit", e))?;
            let result = f
                .bind(py)
                .call1((self.state.bind(py), t, dt, args))
                .map_err(|e| self.exception(py, "next_sample_hit", e))?;
            result
                .extract::<f64>()
                .map_err(|e| self.exception(py, "next_sample_hit", e))
        })
    }

    /// The `xc`-aware counterpart to [`Self::next_sample_hit`] -- calls `next_sample_hit(state,
    /// t, dt, inputs, xc)`.
    ///
    /// # Panics
    /// Same as [`Self::next_sample_hit`].
    pub fn next_sample_hit_xc(
        &self,
        t: f64,
        dt: f64,
        inputs: &[PyInput],
        xc: &[f64],
    ) -> Result<f64, PyBlockError> {
        if self.next_sample_hit_fn.is_none() {
            panic!(
                "next_sample_hit_xc requires a .py file defining next_sample_hit (required for \
                 ts=variable — this should be unreachable, since that's checked up front)"
            );
        }
        Python::attach(|py| {
            let f = self.next_sample_hit_fn.as_ref().unwrap().clone_ref(py);
            let args =
                build_args(py, inputs).map_err(|e| self.exception(py, "next_sample_hit", e))?;
            let xc_arr = numpy::PyArray1::from_slice(py, xc);
            let result = f
                .bind(py)
                .call1((self.state.bind(py), t, dt, args, xc_arr))
                .map_err(|e| self.exception(py, "next_sample_hit", e))?;
            result
                .extract::<f64>()
                .map_err(|e| self.exception(py, "next_sample_hit", e))
        })
    }

    /// Evaluates `derivative(state, t, inputs, xc)` once: given the current continuous-state
    /// vector `xc` and this step's already-resolved inputs, returns `dxc/dt`. Must not mutate
    /// `state` (documented, not enforced, same as `cscript`'s equivalent contract).
    ///
    /// # Panics
    /// If this instance doesn't have a `derivative` function resolved -- unreachable in
    /// practice, since [`PyBlockRegistry::instantiate_xc`] already requires it up front.
    pub fn derivative(
        &self,
        t: f64,
        inputs: &[PyInput],
        xc: &[f64],
    ) -> Result<Vec<f64>, PyBlockError> {
        if self.derivative_fn.is_none() {
            panic!(
                "derivative requires an instance created via PyBlockRegistry::instantiate_xc \
                 (which already requires `derivative` up front -- this should be unreachable)"
            );
        }
        Python::attach(|py| {
            let derivative_fn = self.derivative_fn.as_ref().unwrap().clone_ref(py);
            let args = build_args(py, inputs).map_err(|e| self.exception(py, "derivative", e))?;
            let xc_arr = numpy::PyArray1::from_slice(py, xc);
            let result = derivative_fn
                .bind(py)
                .call1((self.state.bind(py), t, args, xc_arr))
                .map_err(|e| self.exception(py, "derivative", e))?;
            extract_vector(result).map_err(|e| self.exception(py, "derivative", e))
        })
    }

    /// RK4-integrates `xc` forward by `dt`, holding `t`/`inputs` fixed across all four stages --
    /// the same zero-order-hold convention `cscript_ffi::CScriptInstance::rk4_step_xc`/
    /// `continuous_blocks::StateSpace::rk4_step` both use for every other dynamic block.
    pub fn rk4_step_xc(
        &self,
        xc: &[f64],
        t: f64,
        inputs: &[PyInput],
        dt: f64,
    ) -> Result<Vec<f64>, PyBlockError> {
        let add = |a: &[f64], b: &[f64], scale: f64| -> Vec<f64> {
            a.iter().zip(b).map(|(ai, bi)| ai + scale * bi).collect()
        };
        let k1 = self.derivative(t, inputs, xc)?;
        let k2 = self.derivative(t, inputs, &add(xc, &k1, dt / 2.0))?;
        let k3 = self.derivative(t, inputs, &add(xc, &k2, dt / 2.0))?;
        let k4 = self.derivative(t, inputs, &add(xc, &k3, dt))?;
        Ok((0..xc.len())
            .map(|i| xc[i] + (dt / 6.0) * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]))
            .collect())
    }

    /// A real, independent deep copy of this instance's state via Python's own generic
    /// `copy.deepcopy` -- no author opt-in required, unlike `cscript_clone` (see the module doc
    /// comment). `Err` if the state genuinely can't be deep-copied (e.g. an open file handle),
    /// a normal Python exception, not a panic.
    pub fn try_clone(&self) -> Result<PyBlockInstance, PyBlockError> {
        Python::attach(|py| {
            let module = exec_fresh_module(py, &self.path, &self.source).map_err(|e| {
                PyBlockError::Load {
                    path: self.path.clone(),
                    message: py_traceback(py, &e),
                }
            })?;
            let get = |name: &'static str| -> Option<Py<PyAny>> {
                module.getattr(name).ok().map(|f| f.unbind())
            };
            let copy_mod = py
                .import("copy")
                .map_err(|e| self.exception(py, "try_clone", e))?;
            let cloned_state = copy_mod
                .call_method1("deepcopy", (self.state.bind(py),))
                .map_err(|e| self.exception(py, "try_clone", e))?
                .unbind();
            Ok(PyBlockInstance {
                path: self.path.clone(),
                source: self.source.clone(),
                state: cloned_state,
                output_fn: get("output"),
                derivative_fn: get("derivative"),
                output_xc_fn: get("output_xc"),
                update_fn: get("update"),
                next_sample_hit_fn: get("next_sample_hit"),
            })
        })
    }

    /// This instance's state object as `pickle` bytes -- the checkpoint counterpart of
    /// [`Self::try_clone`], same no-author-opt-in reasoning: anything `copy.deepcopy` accepts is
    /// almost always picklable, and the exceptions (an open handle, a lambda held in state)
    /// surface as an ordinary Python exception naming this block's file, never a panic. The
    /// module's functions are *not* serialized -- a resumed run re-executes the block's own
    /// `.py` file and only swaps the state object back in (see [`Self::restore_state`]).
    pub fn pickle_state(&self) -> Result<Vec<u8>, PyBlockError> {
        Python::attach(|py| {
            let pickle = py
                .import("pickle")
                .map_err(|e| self.exception(py, "pickle_state", e))?;
            let bytes = pickle
                .call_method1("dumps", (self.state.bind(py),))
                .map_err(|e| self.exception(py, "pickle_state", e))?;
            bytes
                .extract::<Vec<u8>>()
                .map_err(|e| self.exception(py, "pickle_state", e))
        })
    }

    /// Replaces this instance's state object with the unpickled `bytes` -- the load half of
    /// [`Self::pickle_state`]. The instance itself (its module, its functions) is the one the
    /// netlist built; only the state it carries changes.
    pub fn restore_state(&mut self, bytes: &[u8]) -> Result<(), PyBlockError> {
        Python::attach(|py| {
            let pickle = py
                .import("pickle")
                .map_err(|e| self.exception(py, "restore_state", e))?;
            let state = pickle
                .call_method1("loads", (pyo3::types::PyBytes::new(py, bytes),))
                .map_err(|e| self.exception(py, "restore_state", e))?;
            self.state = state.unbind();
            Ok(())
        })
    }

    fn exception(&self, py: Python<'_>, function: &'static str, err: PyErr) -> PyBlockError {
        PyBlockError::Exception {
            path: self.path.clone(),
            function,
            message: py_traceback(py, &err),
        }
    }
}

impl Clone for PyBlockInstance {
    /// # Panics
    /// If `copy.deepcopy` itself fails on this instance's own state -- see [`Self::try_clone`]
    /// for the non-panicking form.
    fn clone(&self) -> Self {
        self.try_clone()
            .unwrap_or_else(|e| panic!("cannot clone this PyBlockInstance: {e}"))
    }
}

fn exec_fresh_module<'py>(
    py: Python<'py>,
    path: &Path,
    source: &str,
) -> PyResult<pyo3::Bound<'py, PyModule>> {
    let code = CString::new(source)?;
    let filename = CString::new(path.to_string_lossy().as_bytes())?;
    let module_name = CString::new(
        path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "pyblock".to_string()),
    )?;
    PyModule::from_code(py, &code, &filename, &module_name)
}

/// A genuinely separate, additive contract from [`PyBlockInstance`] above — never modifies or
/// reuses its own lifecycle (no `start`, no `state`, no `t`/`dt`), for a plain, pure, stateless
/// Python function: the closest equivalent to a named-input/named-output function block in
/// other block-diagram tools (`function [Y1, Y2] = f(X1, X2) ... end`-style):
///
/// ```python
/// def compute_gate_pattern(phase_degrees):
///     phase = phase_degrees % 360
///     if phase == 0:
///         return 9, 6
///     elif 0 < phase < 180:
///         return 2066, 1057
///     # ...
///     return AQCTLA, AQCTLB
/// ```
///
/// [`PyFunctionRegistry::instantiate`] resolves whichever function name is given and calls it
/// **positionally** (`f(*inputs)`, one declared `inputs=` entry per positional argument — never
/// bundled into one list the way [`PyBlockInstance::call`]'s own `inputs` parameter is), so a
/// function with N named parameters, one with `*args`, or any mix, all work unchanged: there is
/// no parameter-*name* matching against the netlist's own signal names, only positional order,
/// the same way every other block in this graph maps `inputs=A,B,C` to its own computation by
/// position. A single return value or a tuple/list of them (multiple outputs) both work, via
/// the same [`extract_outputs`] this module's other contract already uses.
///
/// Being genuinely stateless, [`PyFunctionInstance`] is always cheaply, trivially cloneable
/// (just `clone_ref`ing the function handle) — unlike [`PyBlockInstance::try_clone`], there is
/// no `copy.deepcopy` call and no way for it to fail.
mod pure_function {
    use std::path::{Path, PathBuf};

    use pyo3::prelude::*;
    use pyo3::types::PyTuple;

    use super::{exec_fresh_module, extract_outputs, py_traceback, PyBlockError, PyInput};

    // Deliberately not shared with `super::build_args` (which builds a `PyList` bundling every
    // input into *one* list argument, for `PyBlockInstance`'s own `output(state, t, dt,
    // inputs)` contract) -- this module never touches that function or its behavior, only
    // reads the same `PyInput` values a completely independent way (one positional argument
    // per input, via a `PyTuple`).
    fn build_positional_args<'py>(
        py: Python<'py>,
        inputs: &[PyInput],
    ) -> PyResult<pyo3::Bound<'py, PyTuple>> {
        use numpy::PyArray1;
        let mut items: Vec<Py<PyAny>> = Vec::with_capacity(inputs.len());
        for input in inputs {
            let obj: Py<PyAny> = match input {
                PyInput::Scalar(x) => x.into_pyobject(py)?.into_any().unbind(),
                PyInput::Vector(xs) => PyArray1::from_slice(py, xs).into_any().unbind(),
            };
            items.push(obj);
        }
        PyTuple::new(py, items)
    }

    /// Caches each `.py` file's own source text by path, same convention as
    /// [`super::PyBlockRegistry`] — a fresh [`PyFunctionInstance`] execs it into its own new
    /// namespace per instance.
    #[derive(Default)]
    pub struct PyFunctionRegistry {
        sources: std::collections::BTreeMap<PathBuf, String>,
    }

    impl PyFunctionRegistry {
        pub fn new() -> Self {
            Self::default()
        }

        fn load_source(&mut self, path: &Path) -> Result<String, PyBlockError> {
            if let Some(source) = self.sources.get(path) {
                return Ok(source.clone());
            }
            let source = std::fs::read_to_string(path).map_err(|e| PyBlockError::Load {
                path: path.to_path_buf(),
                message: e.to_string(),
            })?;
            self.sources.insert(path.to_path_buf(), source.clone());
            Ok(source)
        }

        /// Loads `path` if not already cached, resolves `function_name` as a callable, and
        /// returns a fresh [`PyFunctionInstance`] — no `start()` call, no state at all.
        pub fn instantiate(
            &mut self,
            path: &Path,
            function_name: &str,
        ) -> Result<PyFunctionInstance, PyBlockError> {
            let source = self.load_source(path)?;
            PyFunctionInstance::new(path, &source, function_name)
        }
    }

    #[derive(Debug)]
    pub struct PyFunctionInstance {
        path: PathBuf,
        function_name: String,
        function: Py<PyAny>,
    }

    impl PyFunctionInstance {
        fn new(path: &Path, source: &str, function_name: &str) -> Result<Self, PyBlockError> {
            Python::attach(|py| {
                let module =
                    exec_fresh_module(py, path, source).map_err(|e| PyBlockError::Load {
                        path: path.to_path_buf(),
                        message: py_traceback(py, &e),
                    })?;
                let function = module
                    .getattr(function_name)
                    .map_err(|_| PyBlockError::MissingFunction {
                        path: path.to_path_buf(),
                        name: "function",
                        message: format!("`{function_name}` not found"),
                    })?
                    .unbind();
                Ok(PyFunctionInstance {
                    path: path.to_path_buf(),
                    function_name: function_name.to_string(),
                    function,
                })
            })
        }

        /// Calls the function positionally with `inputs`, in declared order (`f(*inputs)`),
        /// returning `out_len` values.
        pub fn call(&self, inputs: &[PyInput], out_len: usize) -> Result<Vec<f64>, PyBlockError> {
            Python::attach(|py| {
                let args = build_positional_args(py, inputs).map_err(|e| self.exception(py, e))?;
                let result = self
                    .function
                    .bind(py)
                    .call1(args)
                    .map_err(|e| self.exception(py, e))?;
                extract_outputs(result, out_len).map_err(|e| self.exception(py, e))
            })
        }

        fn exception(&self, py: Python<'_>, err: PyErr) -> PyBlockError {
            PyBlockError::Exception {
                path: self.path.clone(),
                function: "call",
                message: format!("in `{}`: {}", self.function_name, py_traceback(py, &err)),
            }
        }
    }

    impl Clone for PyFunctionInstance {
        fn clone(&self) -> Self {
            Python::attach(|py| PyFunctionInstance {
                path: self.path.clone(),
                function_name: self.function_name.clone(),
                function: self.function.clone_ref(py),
            })
        }
    }
}
pub use pure_function::{PyFunctionInstance, PyFunctionRegistry};
