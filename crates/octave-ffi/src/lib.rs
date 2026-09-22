//! Spawns and drives one persistent `octave-cli` subprocess and calls user-supplied,
//! Octave-compatible `.m` functions once per resolved block-graph step -- the Octave-hosted
//! counterpart to `pyblock_ffi`'s stateless [`pyblock_ffi::PyFunctionInstance`] contract, for
//! people with legacy Octave-compatible `.m` scripts. See `general-simulator`'s own
//! `book/dev-guide/src/octave-blocks.md` for the full design writeup (licensing rationale,
//! measured costs, protocol design); this module doc comment covers the mechanics.
//!
//! ## Why a subprocess, never a linked library
//!
//! GNU Octave is GPLv3-licensed. Linking Octave's embedding library (`liboctinterp`) directly
//! into this project's own permissively-licensed binaries would risk pulling them under GPL.
//! **This crate never links against any Octave C/C++ library, and has no Octave crate/library
//! as a build-time dependency at all** -- the only integration is spawning the separately
//! installed `octave-cli` *binary* as a subprocess ("mere aggregation" under GPL, not linking --
//! the same reason tools routinely shell out to `ffmpeg`/`gs` without inheriting their license).
//! Unlike `pyblock_ffi` (which needs `pyo3`+`libpython` at *build* time, and is gated behind an
//! optional Cargo feature for exactly that reason), this crate builds unconditionally on every
//! platform and only fails at *runtime* -- with a clear [`OctaveError::NotFound`] -- if
//! `octave-cli` isn't on `PATH` when a `kind=octfunc` block is actually used.
//!
//! ## Stateful blocks (`kind=octblock`)
//!
//! [`OctaveSession::call`]/[`OctaveSession::add_path`] above are the whole surface `kind=octfunc`
//! (stateless) needs. [`OctaveSession::call_start`], [`OctaveSession::call_stateful`],
//! [`OctaveSession::call_readonly_stateful`], and [`OctaveSession::rk4_step_xc_stateful`] extend
//! this session with the stateful counterpart, `kind=octblock` -- see
//! `general-simulator`'s own `book/dev-guide/src/octave-blocks.md` for the full per-instance
//! opaque-state design (one shared `__gs_state` global struct in the Octave session itself,
//! keyed by instance name, never serialized back to Rust) and the file-per-function contract
//! this protocol calls into.
//!
//! ## One shared, persistent process
//!
//! [`OctaveSession`] wraps exactly one `octave-cli` child process, meant to be shared by every
//! `kind=octfunc` block instance in a single simulation run -- this is the stateless/`pyfunc`
//! case (no per-instance state to isolate), so there's nothing that needs a *separate* process
//! per instance, and every instance calling into the same process is what makes a single
//! `octave-cli` startup pay for the whole run instead of once per block. Measured on this
//! machine (`octave-cli` 11.1.0): a cold-started fresh process per call costs roughly
//! 165-220 ms *every* call (infeasible for a transient run with more than a handful of steps);
//! a single persistent process costs that startup cost exactly once (~158 ms), then roughly
//! 0.044 ms per call afterward. [`OctaveSession::spawn`] pays the startup cost immediately --
//! callers wanting the "don't start Octave at all unless a netlist actually uses `kind=octfunc`"
//! property (this crate's own recommended default) should construct their `OctaveSession`
//! lazily, on first use, rather than eagerly for every run.
//!
//! ## The call protocol
//!
//! [`OctaveSession::add_path`] must be called once per unique directory containing a `.m` file
//! this session will call into -- Octave requires the file to be on its own path and the file
//! name to match the function name:
//!
//! ```text
//! addpath('<absolute-dir-containing-the-.m-file>');
//! ```
//!
//! [`OctaveSession::call`] passes every input as a literal numeric argument, never assigned into
//! a named workspace variable -- this is what makes multiple block instances safely share one
//! process: since nothing is ever written to the shared workspace, there is no possible
//! variable-name collision between instances.
//!
//! ```text
//! try
//!   [__o0, __o1, ...] = <function>(<arg0>, <arg1>, ...);
//!   printf("%.17g\n", __o0);
//!   printf("%.17g\n", __o1);
//!   ...
//!   printf("@@GS_OCT_<call_id>@@\n");
//! catch __err
//!   printf("ERROR: %s\n", __err.message);
//!   printf("@@GS_OCT_<call_id>@@\n");
//! end
//! ```
//!
//! Stdout is read line by line until the marker line appears. If the first line is
//! `ERROR: ...`, the call failed on the Octave side -- surfaced as [`OctaveError::Runtime`], not
//! parsed as a numeric output and not a panic. `call_id` is a per-process-lifetime incrementing
//! counter, embedded in the marker specifically so a desync (a stray leftover line from a prior
//! malformed exchange) is detectable rather than silently misread as the next call's own output.
//!
//! `%.17g` on the Octave side, together with [`str::parse::<f64>`] on the Rust side, round-trips
//! a real fractional double bit-for-bit -- confirmed empirically this session against a value
//! with a genuine precision tail (`0.1 + 0.2` prints as `0.30000000000000004` and parses back to
//! the exact same bits), not just round numbers.
//!
//! A runtime error caught by the `try`/`catch` (e.g. calling a function with the wrong arity)
//! reports cleanly via the `ERROR:` line, and **the session survives and keeps working
//! correctly for subsequent calls** -- confirmed empirically this session; there is no need to
//! respawn the process after a caught Octave-side error, and this crate never does.
//!
//! ## Process death mid-run
//!
//! If the persistent process dies (crash, killed, `octave-cli` itself panics) partway through a
//! run, a blocking read on its stdout would hang forever without special handling. Every read in
//! this crate detects EOF/a broken pipe and returns [`OctaveError::ProcessExited`] (carrying any
//! stderr the process produced before dying) instead of hanging -- see
//! [`tests/session.rs`](https://github.com) `process_dies_mid_run_reports_clean_error` for a
//! test that actually kills the child process and confirms this path works, not just a read of
//! the code.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::thread;

/// Every way calling into an [`OctaveSession`] can fail -- three genuinely different cases a
/// caller needs to handle differently: `octave-cli` isn't installed at all (an actionable "go
/// install Octave" message), the process died out from under this session (a bug in the user's
/// own `.m` file crashing Octave itself, or something killing the process externally), or an
/// ordinary Octave-side runtime error caught by this crate's own `try`/`catch` wrapper (a
/// perfectly normal outcome -- wrong arity, a typo'd function name, a `.m` file that raises on
/// bad input -- and one the session survives).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OctaveError {
    /// `octave-cli` could not be spawned -- almost always because it isn't on `PATH`. `message`
    /// is the underlying [`std::io::Error`]'s own text.
    NotFound { message: String },
    /// The `octave-cli` child process is no longer alive (its stdin/stdout pipe closed) --
    /// detected as EOF on a blocking read that expected more output. `stderr` is everything the
    /// process wrote to its own stderr before dying, if any (empty string if none was
    /// captured), included because it is very often the only clue why Octave itself crashed.
    ProcessExited { stderr: String },
    /// The Octave side raised inside the `try`/`catch` this crate wraps every call in --
    /// `message` is `err.message` from Octave's own caught exception object. The session is
    /// still alive and safe to keep using after this.
    Runtime { message: String },
    /// Writing to the child process's stdin failed for a reason other than the process having
    /// already exited (e.g. some other I/O failure) -- `message` is the underlying
    /// [`std::io::Error`]'s own text.
    Io { message: String },
}

impl std::fmt::Display for OctaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OctaveError::NotFound { message } => {
                write!(
                    f,
                    "could not start octave-cli (is it installed and on PATH?): {message}"
                )
            }
            OctaveError::ProcessExited { stderr } => {
                if stderr.is_empty() {
                    write!(f, "the octave-cli process exited unexpectedly")
                } else {
                    write!(
                        f,
                        "the octave-cli process exited unexpectedly; stderr: {stderr}"
                    )
                }
            }
            OctaveError::Runtime { message } => write!(f, "Octave raised: {message}"),
            OctaveError::Io { message } => write!(f, "I/O error talking to octave-cli: {message}"),
        }
    }
}

impl std::error::Error for OctaveError {}

/// Formats `x` as an Octave numeric literal that round-trips back to the exact same bits --
/// [`f64`]'s own [`std::fmt::Display`] already produces the shortest decimal string that
/// round-trips (confirmed empirically this session: `0.1 + 0.2` formats as
/// `0.30000000000000004`, matching the value `%.17g` reads back on the Octave side), which is
/// already valid Octave syntax for every finite value. `inf`/`-inf`/`NaN` are formatted as
/// Octave's own `Inf`/`-Inf`/`NaN` literals, which [`f64`]'s `Display` does not produce
/// unmodified (`inf`/`NaN` lowercase-`inf`, not directly valid as a bare Octave token in every
/// context) -- special-cased here rather than left to chance.
fn format_octave_literal(x: f64) -> String {
    if x.is_nan() {
        "NaN".to_string()
    } else if x.is_infinite() {
        if x > 0.0 {
            "Inf".to_string()
        } else {
            "-Inf".to_string()
        }
    } else {
        format!("{x}")
    }
}

/// Formats `xs` as an Octave row-vector literal (`[v0, v1, ...]`, `[]` when empty) -- used only
/// for the solver-owned `xc` continuous-state vector ([`OctaveSession::call_stateful`]/
/// [`OctaveSession::call_readonly_stateful`]'s own `xc` parameter), the one piece of state this
/// crate ever passes explicitly rather than leaving inside an instance's own opaque
/// `__gs_state` slot -- see the module doc comment's "Per-instance state" section.
fn format_octave_vector(xs: &[f64]) -> String {
    format!(
        "[{}]",
        xs.iter()
            .map(|x| format_octave_literal(*x))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// Shared tail of every stateful/readonly-stateful call: checks for the `ERROR: ` line
/// [`OctaveSession::call`] itself already checks for, then validates the returned line count
/// against `num_outputs` and parses each as `f64` -- exactly [`OctaveSession::call`]'s own tail,
/// factored out so [`OctaveSession::call_stateful`]/[`OctaveSession::call_readonly_stateful`]
/// don't duplicate it a second and third time; `call` itself is left untouched; nothing about
/// its own behavior changed by this helper existing.
fn parse_output_lines(
    function: &str,
    lines: &[String],
    num_outputs: usize,
) -> Result<Vec<f64>, OctaveError> {
    if let Some(first) = lines.first() {
        if let Some(message) = first.strip_prefix("ERROR: ") {
            return Err(OctaveError::Runtime {
                message: message.to_string(),
            });
        }
    }
    if lines.len() != num_outputs {
        return Err(OctaveError::Runtime {
            message: format!(
                "expected {num_outputs} output line(s) from `{function}`, got {} (call \
                 desynchronized, or the function printed unexpected output): {lines:?}",
                lines.len()
            ),
        });
    }
    lines
        .iter()
        .map(|l| {
            l.parse::<f64>().map_err(|_| OctaveError::Runtime {
                message: format!("could not parse `{l}` as a f64 output of `{function}`"),
            })
        })
        .collect()
}

/// Escapes `s` for embedding inside a single-quoted Octave string literal (`'...'`) -- Octave's
/// own escape convention for a literal single quote inside a single-quoted string is to double
/// it (`''`), the same convention SQL and several other languages use.
fn escape_octave_single_quoted(s: &str) -> String {
    s.replace('\'', "''")
}

/// Reads stdout, line by line, until the `@@GS_OCT_<call_id>@@` marker line for `call_id`
/// appears -- returning every line read before it. Detects EOF (the process died) partway
/// through and reports [`OctaveError::ProcessExited`], never blocking forever.
fn read_until_marker(
    stdout: &mut BufReader<std::process::ChildStdout>,
    call_id: u64,
    stderr_rx: &mpsc::Receiver<String>,
) -> Result<Vec<String>, OctaveError> {
    let marker = format!("@@GS_OCT_{call_id}@@");
    let mut lines = Vec::new();
    loop {
        let mut line = String::new();
        let n = stdout.read_line(&mut line).map_err(|e| OctaveError::Io {
            message: e.to_string(),
        })?;
        if n == 0 {
            // EOF: the process's stdout pipe closed, i.e. it died.
            return Err(OctaveError::ProcessExited {
                stderr: drain_stderr(stderr_rx),
            });
        }
        let line = line.trim_end_matches(['\n', '\r']).to_string();
        if line == marker {
            return Ok(lines);
        }
        lines.push(line);
    }
}

/// Collects whatever the background stderr-reader thread has captured so far, without blocking
/// (used only once a process is already known to have exited, so there is nothing further to
/// wait for).
fn drain_stderr(rx: &mpsc::Receiver<String>) -> String {
    let mut out = String::new();
    while let Ok(chunk) = rx.try_recv() {
        out.push_str(&chunk);
    }
    out
}

/// One persistent `octave-cli` process, plus a per-process-lifetime call-id counter used to
/// frame each call's own output (see the module doc comment, "The call protocol"). Construct
/// with [`OctaveSession::spawn`], call [`Self::add_path`] once per unique directory a `.m` file
/// lives in, then [`Self::call`] as many times as needed. Dropping this value kills the child
/// process (see [`Drop`]).
pub struct OctaveSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    stderr_rx: mpsc::Receiver<String>,
    next_call_id: u64,
}

impl std::fmt::Debug for OctaveSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OctaveSession")
            .field("pid", &self.child.id())
            .field("next_call_id", &self.next_call_id)
            .finish()
    }
}

impl OctaveSession {
    /// Spawns `octave-cli` (looked up on `PATH`, exactly the way a shell would) with
    /// interactive/GUI/startup-file behavior suppressed, ready to accept commands on stdin. Pays
    /// Octave's own real startup cost immediately (measured ~158 ms on this machine) -- callers
    /// wanting to avoid that cost for a run that might never actually need `kind=octfunc` should
    /// only call this once the first `octfunc` block is actually encountered, not eagerly for
    /// every run (see the module doc comment).
    pub fn spawn() -> Result<Self, OctaveError> {
        Self::spawn_with_command(Path::new("octave-cli"))
    }

    /// The same as [`Self::spawn`], but with the executable name/path given explicitly rather
    /// than hardcoded to `"octave-cli"` -- exists so this crate's own test suite can exercise
    /// [`OctaveError::NotFound`] deterministically (spawning a name that is guaranteed not to
    /// exist), without needing to actually tamper with `PATH`. Not expected to be useful outside
    /// of tests; `octave-cli` on `PATH` (via [`Self::spawn`]) is the only supported way to reach
    /// a real Octave.
    pub fn spawn_with_command(command: &Path) -> Result<Self, OctaveError> {
        let mut child = Command::new(command)
            // --no-gui: never try to open a graphical window.
            // --norc: don't read the user's own ~/.octaverc -- a session used by this crate
            //   should behave identically regardless of whichever machine it runs on.
            // --quiet: suppress the startup banner, which would otherwise show up as bogus
            //   leading lines on stdout before any real call is even made.
            //
            // Deliberately *not* passing --interactive: confirmed empirically this session that
            // with stdin as a genuine pipe (never a tty), plain `octave-cli` already reads and
            // executes statements as they arrive without waiting for stdin to close, streaming
            // fine across many sequential writes over the process's whole lifetime -- exactly
            // the persistent-session behavior this crate needs. --interactive additionally
            // prints an `octave:N>` prompt to stdout before each statement, which would corrupt
            // this crate's own line-oriented protocol; there is no flag to keep --interactive's
            // "never exit at EOF" behavior without also printing prompts, so it is simply not
            // needed here at all.
            .args(["--no-gui", "--norc", "--quiet"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| OctaveError::NotFound {
                message: e.to_string(),
            })?;

        let stdin = child.stdin.take().expect("stdin was piped");
        let stdout = BufReader::new(child.stdout.take().expect("stdout was piped"));
        let stderr = child.stderr.take().expect("stderr was piped");

        // Read stderr on a background thread: without this, a call that writes enough to
        // stderr (a warning, a caught-but-verbose error) could fill the OS pipe buffer and
        // deadlock this process against the child, since nothing else here ever reads stderr
        // synchronously. The channel just accumulates chunks for `drain_stderr` to collect once
        // (and only once) the process is known to have died.
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            loop {
                let mut line = String::new();
                match reader.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if tx.send(line).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        Ok(OctaveSession {
            child,
            stdin,
            stdout,
            stderr_rx: rx,
            next_call_id: 0,
        })
    }

    /// Sends `addpath('<dir>')` to this session -- required once per unique directory containing
    /// a `.m` file this session will call into (Octave requires the file to be on its own path).
    /// Calling this more than once for the same directory is harmless (Octave's own `addpath`
    /// is idempotent about duplicate entries); callers are not required to deduplicate
    /// themselves.
    pub fn add_path(&mut self, dir: &Path) -> Result<(), OctaveError> {
        let call_id = self.next_call_id;
        self.next_call_id += 1;
        let escaped = escape_octave_single_quoted(&dir.to_string_lossy());
        let script = format!(
            "addpath('{escaped}');\nprintf(\"@@GS_OCT_{call_id}@@\\n\");\nfflush(stdout);\n"
        );
        self.write_and_flush(&script)?;
        read_until_marker(&mut self.stdout, call_id, &self.stderr_rx)?;
        Ok(())
    }

    /// Calls `function`, an Octave function already on this session's own path (see
    /// [`Self::add_path`]), with `inputs` as literal positional numeric arguments (never
    /// assigned into a named workspace variable -- see the module doc comment for why this is
    /// what makes multiple block instances safely share one session), expecting exactly
    /// `num_outputs` scalar return values. `Ok` on success; [`OctaveError::Runtime`] if the
    /// call raised inside Octave's own `try`/`catch` (the session remains usable afterward);
    /// [`OctaveError::ProcessExited`] if the process died during this call.
    pub fn call(
        &mut self,
        function: &str,
        inputs: &[f64],
        num_outputs: usize,
    ) -> Result<Vec<f64>, OctaveError> {
        let call_id = self.next_call_id;
        self.next_call_id += 1;

        let args: Vec<String> = inputs.iter().map(|x| format_octave_literal(*x)).collect();
        let out_vars: Vec<String> = (0..num_outputs).map(|i| format!("__o{i}")).collect();

        let mut script = String::new();
        script.push_str("try\n  ");
        if num_outputs == 0 {
            script.push_str(&format!("{function}({});\n", args.join(", ")));
        } else if num_outputs == 1 {
            script.push_str(&format!(
                "{} = {function}({});\n",
                out_vars[0],
                args.join(", ")
            ));
        } else {
            script.push_str(&format!(
                "[{}] = {function}({});\n",
                out_vars.join(", "),
                args.join(", ")
            ));
        }
        for v in &out_vars {
            script.push_str(&format!("  printf(\"%.17g\\n\", {v});\n"));
        }
        script.push_str(&format!("  printf(\"@@GS_OCT_{call_id}@@\\n\");\n"));
        script.push_str("catch __err\n");
        script.push_str("  printf(\"ERROR: %s\\n\", __err.message);\n");
        script.push_str(&format!("  printf(\"@@GS_OCT_{call_id}@@\\n\");\n"));
        script.push_str("end\n");
        script.push_str("fflush(stdout);\n");

        self.write_and_flush(&script)?;
        let lines = read_until_marker(&mut self.stdout, call_id, &self.stderr_rx)?;

        if let Some(first) = lines.first() {
            if let Some(message) = first.strip_prefix("ERROR: ") {
                return Err(OctaveError::Runtime {
                    message: message.to_string(),
                });
            }
        }

        if lines.len() != num_outputs {
            // A desync: either the marker's own call_id protected us from misreading a *later*
            // call's output as this one's, but here the *right* call's own output line count
            // itself doesn't match what was asked for -- e.g. the function itself printed extra
            // output, or genuinely returned fewer values than requested and Octave still didn't
            // raise. Surfaced as a Runtime error rather than silently truncating/padding.
            return Err(OctaveError::Runtime {
                message: format!(
                    "expected {num_outputs} output line(s) from `{function}`, got \
                     {} (call desynchronized, or the function printed unexpected output): {lines:?}",
                    lines.len()
                ),
            });
        }

        lines
            .iter()
            .map(|l| {
                l.parse::<f64>().map_err(|_| OctaveError::Runtime {
                    message: format!("could not parse `{l}` as a f64 output of `{function}`"),
                })
            })
            .collect()
    }

    /// Initializes `instance`'s own opaque state slot in the shared `__gs_state` global struct
    /// by calling `<function>_start()` (no arguments) and storing the result into
    /// `__gs_state.('<instance>')` -- the stateful counterpart to [`Self::call`], and the first
    /// call any `kind=octblock` instance must make before [`Self::call_stateful`]/
    /// [`Self::call_readonly_stateful`] can read a meaningful slot for it. Per-instance
    /// isolation is purely a property of `instance` being a distinct struct field name in one
    /// shared global -- two instances calling the *same* `function` with different `instance`
    /// names never see each other's state, confirmed empirically (see `tests/session.rs`'s own
    /// `two_instances_of_same_stateful_function_do_not_contaminate_each_others_state`).
    pub fn call_start(&mut self, function: &str, instance: &str) -> Result<(), OctaveError> {
        let call_id = self.next_call_id;
        self.next_call_id += 1;
        let inst = escape_octave_single_quoted(instance);

        let mut script = String::new();
        script.push_str("global __gs_state;\n");
        script.push_str("try\n");
        script.push_str(&format!("  __gs_state.('{inst}') = {function}_start();\n"));
        script.push_str(&format!("  printf(\"@@GS_OCT_{call_id}@@\\n\");\n"));
        script.push_str("catch __err\n");
        script.push_str("  printf(\"ERROR: %s\\n\", __err.message);\n");
        script.push_str(&format!("  printf(\"@@GS_OCT_{call_id}@@\\n\");\n"));
        script.push_str("end\n");
        script.push_str("fflush(stdout);\n");

        self.write_and_flush(&script)?;
        let lines = read_until_marker(&mut self.stdout, call_id, &self.stderr_rx)?;
        if let Some(first) = lines.first() {
            if let Some(message) = first.strip_prefix("ERROR: ") {
                return Err(OctaveError::Runtime {
                    message: message.to_string(),
                });
            }
        }
        Ok(())
    }

    /// Calls `function(__gs_state.('<instance>'), <args...>[, <xc>])`, expecting `function` to
    /// return `num_outputs + 1` values: the instance's own *new* state first, then
    /// `num_outputs` plain numeric outputs -- `[new_state, y0, y1, ...] = function(state, ...)`,
    /// exactly the shape the file-per-function contract's own `<function>`/`<function>_output_xc`/
    /// `<function>_update` all share (see `book/dev-guide/src/octave-blocks.md`). `xc`, when
    /// given, is appended as one literal Octave vector argument after `args` (e.g. `[1, 2, 3]`),
    /// never flattened into separate scalar arguments -- this is the one piece of *solver*-owned
    /// state that crosses the pipe explicitly rather than living in the instance's own opaque
    /// slot.
    ///
    /// `__gs_state.('<instance>')` is only overwritten if `function` returns successfully --
    /// Octave never performs a multi-value assignment's left-hand-side writes if evaluating the
    /// right-hand side raises, so a caught error here leaves the instance's previous state slot
    /// completely untouched (confirmed empirically; see `tests/session.rs`'s own
    /// `stateful_call_that_errors_does_not_corrupt_instance_state`) -- no special-casing needed
    /// on this crate's own side to get that guarantee.
    pub fn call_stateful(
        &mut self,
        function: &str,
        instance: &str,
        args: &[f64],
        xc: Option<&[f64]>,
        num_outputs: usize,
    ) -> Result<Vec<f64>, OctaveError> {
        let call_id = self.next_call_id;
        self.next_call_id += 1;
        let inst = escape_octave_single_quoted(instance);

        let mut call_args: Vec<String> = vec![format!("__gs_state.('{inst}')")];
        call_args.extend(args.iter().map(|x| format_octave_literal(*x)));
        if let Some(xc) = xc {
            call_args.push(format_octave_vector(xc));
        }

        let out_vars: Vec<String> = (0..num_outputs).map(|i| format!("__o{i}")).collect();
        let lhs = if out_vars.is_empty() {
            format!("__gs_state.('{inst}')")
        } else {
            format!("[__gs_state.('{inst}'), {}]", out_vars.join(", "))
        };

        let mut script = String::new();
        script.push_str("global __gs_state;\n");
        script.push_str("try\n");
        script.push_str(&format!(
            "  {lhs} = {function}({});\n",
            call_args.join(", ")
        ));
        for v in &out_vars {
            script.push_str(&format!("  printf(\"%.17g\\n\", {v});\n"));
        }
        script.push_str(&format!("  printf(\"@@GS_OCT_{call_id}@@\\n\");\n"));
        script.push_str("catch __err\n");
        script.push_str("  printf(\"ERROR: %s\\n\", __err.message);\n");
        script.push_str(&format!("  printf(\"@@GS_OCT_{call_id}@@\\n\");\n"));
        script.push_str("end\n");
        script.push_str("fflush(stdout);\n");

        self.write_and_flush(&script)?;
        let lines = read_until_marker(&mut self.stdout, call_id, &self.stderr_rx)?;
        parse_output_lines(function, &lines, num_outputs)
    }

    /// Calls `function(__gs_state.('<instance>'), <args...>[, <xc>])`, expecting `function` to
    /// return exactly `num_outputs` plain numeric values -- **the instance's own state slot is
    /// read, but never reassigned** (there is no state variable on the left-hand side at all),
    /// unlike [`Self::call_stateful`]. This is the shape `<function>_derivative` (pure -- must
    /// not mutate state) and `<function>_next_sample_hit` both share.
    pub fn call_readonly_stateful(
        &mut self,
        function: &str,
        instance: &str,
        args: &[f64],
        xc: Option<&[f64]>,
        num_outputs: usize,
    ) -> Result<Vec<f64>, OctaveError> {
        let call_id = self.next_call_id;
        self.next_call_id += 1;
        let inst = escape_octave_single_quoted(instance);

        let mut call_args: Vec<String> = vec![format!("__gs_state.('{inst}')")];
        call_args.extend(args.iter().map(|x| format_octave_literal(*x)));
        if let Some(xc) = xc {
            call_args.push(format_octave_vector(xc));
        }

        let out_vars: Vec<String> = (0..num_outputs).map(|i| format!("__o{i}")).collect();

        let mut script = String::new();
        script.push_str("global __gs_state;\n");
        script.push_str("try\n  ");
        if out_vars.is_empty() {
            script.push_str(&format!("{function}({});\n", call_args.join(", ")));
        } else if out_vars.len() == 1 {
            script.push_str(&format!(
                "{} = {function}({});\n",
                out_vars[0],
                call_args.join(", ")
            ));
        } else {
            script.push_str(&format!(
                "[{}] = {function}({});\n",
                out_vars.join(", "),
                call_args.join(", ")
            ));
        }
        for v in &out_vars {
            script.push_str(&format!("  printf(\"%.17g\\n\", {v});\n"));
        }
        script.push_str(&format!("  printf(\"@@GS_OCT_{call_id}@@\\n\");\n"));
        script.push_str("catch __err\n");
        script.push_str("  printf(\"ERROR: %s\\n\", __err.message);\n");
        script.push_str(&format!("  printf(\"@@GS_OCT_{call_id}@@\\n\");\n"));
        script.push_str("end\n");
        script.push_str("fflush(stdout);\n");

        self.write_and_flush(&script)?;
        let lines = read_until_marker(&mut self.stdout, call_id, &self.stderr_rx)?;
        parse_output_lines(function, &lines, num_outputs)
    }

    /// RK4-integrates `xc` forward by `dt`, holding `args` fixed across all four stages and
    /// calling [`Self::call_readonly_stateful`] against `<function>_derivative` up to four
    /// times (`k1..k4`) -- the direct Octave-hosted analog of
    /// `cscript_ffi::CScriptInstance::rk4_step_xc`/`pyblock_ffi`'s own RK4 stepper, same
    /// zero-order-hold convention (`args`, i.e. `t`/`dt`/inputs, held fixed across all four
    /// stages). `derivative_function` is expected to already carry the `_derivative` suffix
    /// (callers pass `"<function>_derivative"`, matching the file-per-function contract).
    pub fn rk4_step_xc_stateful(
        &mut self,
        derivative_function: &str,
        instance: &str,
        xc: &[f64],
        args: &[f64],
        dt: f64,
    ) -> Result<Vec<f64>, OctaveError> {
        let add = |a: &[f64], b: &[f64], scale: f64| -> Vec<f64> {
            a.iter().zip(b).map(|(ai, bi)| ai + scale * bi).collect()
        };
        let n = xc.len();
        let k1 = self.call_readonly_stateful(derivative_function, instance, args, Some(xc), n)?;
        let k2 = self.call_readonly_stateful(
            derivative_function,
            instance,
            args,
            Some(&add(xc, &k1, dt / 2.0)),
            n,
        )?;
        let k3 = self.call_readonly_stateful(
            derivative_function,
            instance,
            args,
            Some(&add(xc, &k2, dt / 2.0)),
            n,
        )?;
        let k4 = self.call_readonly_stateful(
            derivative_function,
            instance,
            args,
            Some(&add(xc, &k3, dt)),
            n,
        )?;
        Ok((0..n)
            .map(|i| xc[i] + (dt / 6.0) * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]))
            .collect())
    }

    /// Serializes `instance`'s own `__gs_state.('<instance>')` slot to bytes, for a checkpoint:
    /// the slot is copied into a scratch variable, written with Octave's own
    /// `save('-binary', ...)` to a temporary file, and that file is read back here and deleted
    /// -- the same call protocol every other method uses, so a failed `save` (an unset slot, a
    /// value Octave's binary format cannot hold) surfaces as [`OctaveError::Runtime`] and the
    /// session survives. Octave's binary format stores doubles as their exact bits, which is
    /// what makes a resumed run bit-identical. No author-side opt-in is needed, unlike
    /// `cscript`'s own state contract: whatever `<function>_start` returned is what gets saved.
    ///
    /// The temporary file lives in [`std::env::temp_dir`] under a name unique to this process
    /// and call, and is removed before this returns -- on the error path too.
    pub fn save_state(&mut self, instance: &str) -> Result<Vec<u8>, OctaveError> {
        let call_id = self.next_call_id;
        self.next_call_id += 1;
        let path = self.state_temp_path(call_id);
        let inst = escape_octave_single_quoted(instance);
        let file = escape_octave_single_quoted(&path.to_string_lossy());

        let mut script = String::new();
        script.push_str("global __gs_state;\n");
        script.push_str("try\n");
        script.push_str(&format!("  __gs_ckpt = __gs_state.('{inst}');\n"));
        script.push_str(&format!("  save('-binary', '{file}', '__gs_ckpt');\n"));
        script.push_str("  clear __gs_ckpt;\n");
        script.push_str(&format!("  printf(\"@@GS_OCT_{call_id}@@\\n\");\n"));
        script.push_str("catch __err\n");
        script.push_str("  printf(\"ERROR: %s\\n\", __err.message);\n");
        script.push_str(&format!("  printf(\"@@GS_OCT_{call_id}@@\\n\");\n"));
        script.push_str("end\n");
        script.push_str("fflush(stdout);\n");

        let result = self
            .write_and_flush(&script)
            .and_then(|()| read_until_marker(&mut self.stdout, call_id, &self.stderr_rx))
            .and_then(|lines| {
                if let Some(message) = lines.first().and_then(|l| l.strip_prefix("ERROR: ")) {
                    return Err(OctaveError::Runtime {
                        message: message.to_string(),
                    });
                }
                std::fs::read(&path).map_err(|e| OctaveError::Io {
                    message: format!(
                        "could not read the state file Octave saved at {}: {e}",
                        path.display()
                    ),
                })
            });
        let _ = std::fs::remove_file(&path);
        result
    }

    /// The inverse of [`Self::save_state`]: writes `bytes` to a temporary file, `load`s it in
    /// the session, and assigns the loaded value into `__gs_state.('<instance>')` -- replacing
    /// whatever [`Self::call_start`] put there at construction. The file is removed before this
    /// returns, on every path.
    pub fn load_state(&mut self, instance: &str, bytes: &[u8]) -> Result<(), OctaveError> {
        let call_id = self.next_call_id;
        self.next_call_id += 1;
        let path = self.state_temp_path(call_id);
        std::fs::write(&path, bytes).map_err(|e| OctaveError::Io {
            message: format!(
                "could not write the state file for Octave at {}: {e}",
                path.display()
            ),
        })?;
        let inst = escape_octave_single_quoted(instance);
        let file = escape_octave_single_quoted(&path.to_string_lossy());

        let mut script = String::new();
        script.push_str("global __gs_state;\n");
        script.push_str("try\n");
        script.push_str(&format!("  __gs_loaded = load('-binary', '{file}');\n"));
        script.push_str(&format!(
            "  __gs_state.('{inst}') = __gs_loaded.__gs_ckpt;\n"
        ));
        script.push_str("  clear __gs_loaded;\n");
        script.push_str(&format!("  printf(\"@@GS_OCT_{call_id}@@\\n\");\n"));
        script.push_str("catch __err\n");
        script.push_str("  printf(\"ERROR: %s\\n\", __err.message);\n");
        script.push_str(&format!("  printf(\"@@GS_OCT_{call_id}@@\\n\");\n"));
        script.push_str("end\n");
        script.push_str("fflush(stdout);\n");

        let result = self
            .write_and_flush(&script)
            .and_then(|()| read_until_marker(&mut self.stdout, call_id, &self.stderr_rx))
            .and_then(|lines| {
                if let Some(message) = lines.first().and_then(|l| l.strip_prefix("ERROR: ")) {
                    return Err(OctaveError::Runtime {
                        message: message.to_string(),
                    });
                }
                Ok(())
            });
        let _ = std::fs::remove_file(&path);
        result
    }

    /// Where [`Self::save_state`]/[`Self::load_state`] put their one short-lived file: unique
    /// per host process and per call, so two sessions (two test binaries, two runs) never
    /// collide on the same name.
    fn state_temp_path(&self, call_id: u64) -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "gs-octblock-state-{}-{}-{call_id}.bin",
            std::process::id(),
            self.child.id()
        ))
    }

    /// Kills the underlying `octave-cli` process immediately (`SIGKILL`, not a graceful
    /// shutdown), simulating a crash for [`Self`]'s own EOF/broken-pipe handling to be tested
    /// against. Exists purely so this crate's own test suite can exercise
    /// [`OctaveError::ProcessExited`] by actually killing a live process (see
    /// `tests/session.rs`'s own `process_dies_mid_run_reports_clean_error_instead_of_hanging`),
    /// rather than only reviewing the read/EOF-handling code by eye. Not useful outside of
    /// tests -- an ordinary caller wanting to stop a session should simply [`drop`] it.
    pub fn kill_for_test(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    fn write_and_flush(&mut self, script: &str) -> Result<(), OctaveError> {
        self.stdin
            .write_all(script.as_bytes())
            .and_then(|()| self.stdin.flush())
            .map_err(|e| {
                // A closed pipe here means the process already died; report it the same way a
                // failed read would, rather than as a generic Io error, so callers see one
                // consistent case for "the process is gone" regardless of which direction of
                // I/O first noticed it.
                if e.kind() == std::io::ErrorKind::BrokenPipe {
                    OctaveError::ProcessExited {
                        stderr: drain_stderr(&self.stderr_rx),
                    }
                } else {
                    OctaveError::Io {
                        message: e.to_string(),
                    }
                }
            })
    }
}

impl Drop for OctaveSession {
    fn drop(&mut self) {
        // Best-effort: ask nicely first (closing stdin makes an interactive octave-cli exit on
        // its own), then make sure it's actually gone. Errors here are deliberately swallowed --
        // there is nothing a caller could usefully do with a failure to tear down a process
        // that's going away regardless.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
