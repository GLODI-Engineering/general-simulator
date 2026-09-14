# `general-simulator-cli`: the netlist-in/CSV-out runner

The `general-simulator` binary, and the thinnest crate in the workspace by design. Its own
module doc comment (`crates/general-simulator-cli/src/main.rs`) opens with the contract:
"Thin runner for `general-simulator`" — netlist in, CSV waveform to stdout (one row per
resolved timestep, a single row for `--mode dc`), plus `--format raw` and `kind=measure`
post-processing. What it deliberately does **not** do is parse the netlist's device/block
lines itself: "This binary no longer parses any of that itself — `general-mna`'s `build_system`
does, via `general-spice-core`'s real grammar." The old hand-rolled `*`-comment-disguised
parser this crate used to carry (with its `kind=`-substring detection heuristic and
`GateSpec`/`Kind` types) is gone — confirmed by grep against the current `main.rs`, which
contains no `GateSpec` and no local `Kind` enum at all. What remains is a staged pipeline,
each stage a real function in `run()`.

## The pipeline, stage by stage

Everything below is the actual order of `crates/general-simulator-cli/src/main.rs::run`:

1. **Argument parsing** — a plain hand-rolled loop over `env::args()`. Flags:
   `--devices <file>` (optional; default is the netlist itself), `--mode dc|transient`
   (default `dc`), `--tfinal`, `--dt`, the adaptive knobs `--dt-max/--dt-min/--dt-init/
   --reltol/--abstol`, `--max-steps`, `--format csv|raw`, `--out <path>`, `--out-every N`.
   `--dt` and any adaptive flag are mutually exclusive — rejected up front with
   `"--dt (fixed step) and --dt-max/--dt-min/--dt-init/--reltol/--abstol (adaptive step) are
   mutually exclusive — pick one"`. No `--dt` at all means adaptive, with
   `AdaptiveConfig::from_t_final` defaults overridden piecemeal by whichever adaptive flags
   were given. `usage()` spells out the same contract in one string, including the
   `--out-every` rationale (the timestep and the useful output rate can legitimately differ
   by orders of magnitude — a zero-voltage-switching check at 0.4 ps steps across 150 ns is
   3.75 M steps, of which a measurement needs a few thousand).
2. **Netlist read + `kind=measure` stripping** — `measure::extract` recognizes `kind=measure`
   lines and removes them *before* `general_mna::build_system` ever sees the text, replacing
   each with a blank line so every other statement keeps its own line number. Why here:
   `general-mna` has no `kind=measure` entry in its dispatch and would reject the line as an
   unknown device kind; a measurement is evaluated exactly once, after the run, over the
   completed trace, so it has no bearing on topology or per-step evaluation — see
   `measure.rs`'s module doc ("Why this lives here, not in `general-mna`/`dae-runtime`") and
   `book/dev-guide/src/measurements.md` for the full argument.
3. **Build** — `general_mna::build_system` on the stripped source, destructured straight into
   `ideal_diodes`/`ideal_switches`/`gates`/`blocks`/`shared_r_on`. Build failures surface as
   `error: parsing device/block declarations: <message>` (the `{e:?}` of the inner error),
   printed to stderr with exit code 1 by `main()`'s catch-all `error: {message}` handler.
4. **The converter-wired-as-a-node check** — `dae_runtime::reject_sig2phys_wired_into_circuit`
   runs *before* the mode dispatch, so `--mode dc` fails on it too (that path never enters
   the block-graph engine at all). The error is deliberately spelled out rather than left as
   a bare `{e:?}` dump, because the bug being caught was *silent*: a `kind=sig2phys` block
   used as a node on an element line used to build and solve without complaint and report
   `V(<name>) = 0` next to the block's own correct output column. The message names the
   converter, the offending element, and the node, and says what to write instead (name the
   converter in a source's value field or a gate's `ctrl=`, never wire it).
5. **Mode dispatch** —
   - `--mode dc`: `dae_runtime::solve_dc` — but only when the netlist declares **no** ideal
     switch. Every gate is block-driven, and a DC operating point has no notion of a block's
     time-stepped state, so any ideal switch at all fails with
     `device '<name>': every gate is block-driven (gate=block), which needs --mode transient,
     not 'dc' (...)` — see `book/user-guide/src/cli-reference.md`, "--mode dc and block-driven
     gates".
   - `--mode transient`: two genuinely different paths. A netlist with *no ideal switch and
     no block* uses the plain `dae_runtime::simulate_transient` (there is nothing for a
     block-graph step to resolve). Anything else — including a block-only netlist with
     nothing to gate — goes through `run_transient_streamed` and
     `dae_runtime::simulate_transient_with_blocks_streamed`, which "tolerates an empty
     `ideal_switches`/`gates` map or an empty `blocks` slice equally well."
6. **Serialization** — everything lands in a `Waveform` struct, `headers[0]` always the
   time/sweep column, `rows[point][column]` mirroring it. Its own doc comment makes the
   point that matters: both `print_csv` and `raw_format::write_raw` are built from *exactly
   this* struct, so "same data, different serialization" (see
   `book/user-guide/src/reading-output.md`) is structural, not something the two writers
   happen to agree on. `--format csv` (default, unchanged byte-for-byte from before the flag
   existed) prints to stdout; `--format raw` writes a SPICE rawfile to `--out <path>` or
   `<netlist stem>.raw`, because a binary format has nowhere sensible to go on a terminal.
7. **Measurements** — if any `kind=measure` lines were extracted, results go to
   `<netlist stem>.log`, *never* stdout or stderr, matching ngspice/Xyce's own measurement-log
   convention, so stdout stays unconditionally machine-readable: a `kind=measure`-free
   netlist produces byte-for-byte the same stdout as before the feature existed.

## The streaming path (`run_transient_streamed`)

The function's own doc comment records the incident that made streaming mandatory: the
predecessor (`run_transient_with_ideal_switches`) accumulated the whole run in a `Vec`, and a
single large, numerically stiff circuit drove the development machine to ~11 GB RSS / 24 GB
swap before being killed — the write-up lives in an internal phase-shift-PWM modulator
comparison experiment. Now CSV rows go straight to stdout
and raw-format rows straight to a temp file, one step at a time.

Three details survive only because they have to:

- **Headers.** Column names are computable *before* the first row: the block's own name plus
  every extra `output_names` entry (a `cscript`/`pmsm`/`pwm`/`pspwm`/coordinate-transform
  block's secondary outputs), and a vector-valued output expands to `NAME[0]`, `NAME[1]`, ...
  once the first row reveals its length. A block whose output is missing for a point reads
  `NaN` rather than dropping the row.
- **Measurements keep a partial history.** A measurement needs the whole time series of
  whichever signals it names — but only for the columns some `MeasureSpec` actually
  references (`measure::referenced_signals`), never the full circuit's worth: a run with
  measurements on 2 signals out of 70 columns keeps roughly 2/70th of the memory a
  full-`Vec` approach would.
- **The rawfile header dance.** A binary rawfile's `No. Points:` header field is read upfront
  by every reader the format was validated against, but the point count isn't known until the
  run finishes — so row data streams to `<out>.raw.tmp`, then the real header (with the true
  count) is written to the output path followed by a byte-for-byte copy of the temp file,
  which is deleted. Memory cost is the same either way (streamed); disk cost is briefly ~2x
  the final file, "an acceptable trade against never holding row data in memory."

`raw_format.rs`'s module doc is where the rawfile's own rules live: the binary variant of the
format (ASCII header block, then point-major little-endian `f64`s), header fields verified
against the ngspice manual's own worked example and cross-validated against two independent
Python readers (PySpice and `spicelib`), and the variable-type caveat — a block's output has
no "generic signal" type in the format, so non-`V(...)`/`I(...)` columns are tagged `voltage`
as the broadest-compatibility fallback, purely descriptive metadata, value identical to CSV.

`--out-every N` thins the *output*, never the computation: `decimate()` keeps only every
Nth resolved point in the serialized copy, while measurements are still evaluated against
every resolved point ("decimating a measurement's input would change its result rather than its size, which
is a different feature and a worse one").

## Testing strategy: cross-check, not fresh derivation

`crates/general-simulator-cli/tests/cli.rs`'s module doc states the framing explicitly: the
tests spawn the actual built `general-simulator` binary (the standard way to test a CLI)
"against fixture netlists that are the *exact same circuits* already hand-verified in
`dae-runtime`'s own tests — so the expected output here isn't a fresh hand derivation, it's a
cross-check that the CLI's argument parsing, device-file parsing, and CSV formatting
correctly reach the same, already-trusted numerical result." The distinction matters for
what these tests prove: they prove the plumbing, not the numerics. A failure here means "the
already-correct answer didn't make it out of the binary," which is exactly what a runner's
tests should be asserting — the numerical correctness itself is `dae-runtime`'s and
`general-mna`'s burden, already discharged elsewhere (`verification-discipline.md`). The
same file's `dc_mode_reproduces_the_hand_verified_two_diode_operating_point` test asserts
`V(e2) == 7/3` to `1e-9` on a fixture replicating a circuit already hand-verified in
`dae-runtime`'s own suite.

## One stale phrase, found and left in place

The module doc comment is current on everything spot-checked against the authoritative
grammar, `general-mna/src/system_builder.rs`'s `build_kind` match arms (`kind=pwm freq= in=`,
`kind=pspwm f_min= f_max= inputs=freq,phase,duty`, `kind=phys2sig domain=`,
`kind=sig2phys domain= in=`, `kind=cscript ts=/freq=`, `kind=ideal_diode`/`kind=ideal_switch`
— all match). One phrase lags: the module doc calls `kind=statespace` a "single-input
single-output block," but `system_builder.rs`'s own `"statespace"` arm has been genuinely
MIMO since vector signals existed — `b=`/`c=`/`d=` accept matrix forms, `inputs=` takes
exactly `p` entries (each independently scalar or vector, flattened by `dae-runtime`'s own
`evaluate_blocks` into the `u` vector `StateSpace::rk4_step` expects), and a bare scalar `d=`
is rejected once `p>1` or `q>1` with a message telling the author to declare the matrix form.
When the module doc is next edited, that sentence is the one to fix — this chapter states the
current behavior rather than repeating the stale word.

## Source material this was adapted from

- `crates/general-simulator-cli/src/main.rs` — module doc comment, `run()`, `usage()`,
  `run_transient_streamed()`'s doc comment, `decimate()`, `default_log_path()`/
  `default_raw_path()`, the `Waveform`/`OutputFormat` types.
- `crates/general-simulator-cli/src/raw_format.rs` — module doc comment (binary rawfile
  layout, header verification, variable-type caveat).
- `crates/general-simulator-cli/src/measure.rs` — module doc comment (why `kind=measure`
  lives at the CLI level; the `gs-waveform-measurements` dispatch).
- `crates/general-simulator-cli/tests/cli.rs` — module doc comment (cross-check testing
  strategy) and the two-diode/RC fixtures.
- `general-mna/src/system_builder.rs` — the `build_kind` match arms, as the authoritative
  grammar to spot-check the module doc against.
- `book/user-guide/src/reading-output.md` and `cli-reference.md` — the user-facing chapters
  this one links to instead of duplicating.
