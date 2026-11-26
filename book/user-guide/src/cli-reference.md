# Command-line flags

```text
general-simulator <netlist> [--devices <file>] [--mode dc|transient] [--tfinal T]
                   [--dt DT | --dt-max T --dt-min T --dt-init T --reltol R --abstol A]
                   [--max-steps N]
                   [--format csv|raw|samples] [--out <path>] [--out-every N]
                   [--events <path>] [--events-text <path>]
                   [--checkpoint-out <file>] [--checkpoint-every T] [--resume <file>]
general-simulator (--help | -h)
general-simulator (--version | -V)
general-simulator --capabilities
```

`<netlist>` is the one required positional argument — the `.cir` file to run. Every flag below
is optional.

| Flag | Meaning |
| --- | --- |
| `--devices <file>` | Read PWL-device/block declarations from a second file instead of the netlist itself. Rarely needed — see the module doc comment on `crates/general-simulator-cli/src/main.rs` for why the netlist is normally the one, self-contained file. |
| `--mode dc\|transient` | `dc` solves a single operating point. `transient` runs the resolved timestep loop. Default: `dc`. |
| `--tfinal T` | Simulation end time (seconds) for `--mode transient`. Default: `1.0`. |
| `--dt DT` | Fixes the step size every step (deterministic, exactly reproducible). Mutually exclusive with the adaptive-stepping flags below. |
| `--dt-max`/`--dt-min`/`--dt-init` | Override individual defaults of the adaptive step-size controller (`dae_runtime::AdaptiveConfig::from_t_final`), used when `--dt` is omitted. |
| `--reltol`/`--abstol` | Override the adaptive controller's relative/absolute local-truncation-error tolerances. |
| `--max-steps N` | Caps how many accepted adaptive steps one run may take (default `10,000,000`) before aborting with `DaeError::AdaptiveStepStalled`. Each accepted step holds a row in memory for the rest of the run, so a stalled controller would otherwise grow memory without limit. Adaptive stepping only — a `--dt` run takes a precomputed number of steps and cannot stall this way. See [Fixed vs. adaptive time stepping](time-stepping.md). |
| `--format csv\|raw\|samples` | Output serialization — see below. Default: `csv`. |
| `--out <path>` | Destination file for `--format raw` and `--format samples`. Ignored for `--format csv` (CSV always goes to stdout). |
| `--out-every N` | Write only every Nth resolved point. Thins the output, never the computation — see below. Default: `1`. |
| `--events <path>` | Where the JSONL event stream goes. Default: `<netlist stem>.events.jsonl`, beside the netlist. |
| `--events-text <path>` | Where its human-readable twin goes, independently of `--events`. Default: `<netlist stem>.events.txt`. Both files are written only for a netlist that declares rules, and a path that cannot be created is an error — see below. |
| `--checkpoint-out <file>` | Write the run's complete state at `--tfinal` so a later run can `--resume` it. See [Checkpoint and resume](checkpoint-resume.md). |
| `--checkpoint-every T` | With `--checkpoint-out`: also rewrite that file every `T` seconds of simulated time, so a killed run loses at most `T`. |
| `--resume <file>` | Continue from a checkpoint instead of from $t = 0$; `--tfinal` stays absolute. Only loads into the deck it was written from. |
| `--help`, `-h` | Print a grouped summary of every flag, with defaults and three worked invocations, to stdout and exit 0. Asking for help is not a misuse, so it does not go to stderr with a failing status the way a wrong argument does. |
| `--version`, `-V` | Print `general-simulator <version>` to stdout and exit 0. The program name, not the crate's (`general-simulator-cli`). |
| `--capabilities` | Print what *this* binary can run, as `key=value` lines, to stdout and exit 0 — see below. |

All three of those are read before the positional `<netlist>`, because every argument is otherwise a
path to this CLI: before they existed, `general-simulator --version` answered
`error: reading --version: No such file or directory` with status 1. Anything probing the binary
to decide whether it works read that as a broken solver — the VS Code front end's own discovery
did, and rejected every binary it was pointed at.

For the same reason a flag typed *before* the netlist now names itself rather than blaming a
missing file:

```text
$ general-simulator --tfinal 1e-3
error: unrecognized argument '--tfinal', or a flag given before the netlist.
The netlist path comes first: general-simulator <netlist> [OPTIONS]
Run `general-simulator --help` for the full list.
```

`--dt` and any of `--dt-max`/`--dt-min`/`--dt-init`/`--reltol`/`--abstol` are mutually
exclusive — pick fixed-step or adaptive stepping, not both. Omitting `--dt` entirely selects
adaptive stepping, the default, the same local-truncation-error approach every real SPICE-family
tool uses by default when only the run length is given.

## `--capabilities`: what *this* binary can run

```console
$ general-simulator --capabilities
program=general-simulator
version=0.9.0
python=no
octave=subprocess
cscript=load-only
```

**`python` is the one that cannot be answered any other way.** `kind=pyblock` and `kind=pyfunc`
need a `--features python` build, and the release workflow deliberately does **not** publish one
— linking against one specific libpython would make it a worse download. So a user who
downloads a release, or who follows a `cargo build --release -p general-simulator-cli` line in a
front end's own setting description, ends up with an engine that silently excludes the two
most-used extensibility blocks, and until this flag existed the only way to find out was to draw
a sheet, press Run and read `PythonSupportNotCompiledIn`.

`--help` cannot answer it: its NOTES section says *"kind=pyblock and kind=pyfunc need a
`--features python` build of this binary"* whatever this binary is, so a front end scraping that
text learns nothing about the binary in front of it.

To get one that can:

```bash
cargo build --release -p general-simulator-cli --features python
```

which needs a discoverable Python with its shared library.

**The three extensibility languages answer three different kinds of question, and the values say
which kind:**

| key | value | what kind of fact it is |
| --- | --- | --- |
| `python` | `yes` / `no` | a property of this **build**. Nothing but a rebuild changes it. |
| `octave` | `subprocess` | a property of the **design**: `octave-ffi` spawns `octave-cli` and never links against Octave, so this binary is always able to *try*. Whether that program is on `PATH` is a question about the machine, which no build can answer. |
| `cscript` | `load-only` | a property of the **contract**: this binary loads a shared library and never invokes a compiler. A caller that wants a `.so` built has to build it. |

Every line is `key=value`, nothing else — no banner, no prose, no blank lines. That is so a
front end never has to regex text a reword would break, and so a new key can be added without
breaking a reader that does not know it. Parse it in four lines:

```bash
python=$(general-simulator --capabilities | sed -n 's/^python=//p')
[ "$python" = yes ] || echo "this engine cannot run a Python block"
```

## `--out-every`: when the timestep and the useful output rate differ

`--out-every N` writes every Nth resolved point and skips the rest. It affects **serialization
only**:

- every step is still taken, so the solution is bit-identical to the same run without the flag;
- `kind=measure` still sees every point, so measurement results are unchanged (decimating a
  measurement's input would change its *answer*, not just its size);
- `--out-every 1`, the default, is byte-for-byte the behaviour from before the flag existed.

Point 0 is always written. After that it is simply every Nth, so the final point appears only if
its index lands on the stride.

It counts *resolved points*, not time, so under adaptive stepping the spacing of the rows follows
the solver's own step decisions rather than a fixed interval of simulated time
(GLODI-Engineering/general-simulator#110 asks whether a time-based `--out-dt` should exist beside
it). Under `--dt` the two are the same thing.

**It cannot be combined with `--format samples`, and the combination is refused.** Both flags
answer "this run is too big to look at". A stride keeps one point per interval and drops the
rest; the samples format keeps the minimum and maximum of every interval. Applied together the
stride removes the peaks before the pyramid can record them, and what comes out is a container
with a complete pyramid over an incomplete run — indistinguishable from a complete file. See
[Runs too large to open](large-runs.md) for which to reach for.

### When a stride is safe

A stride is safe exactly when it is far narrower than the narrowest feature you intend to see.
The ZVS case below is the safe end by three orders of magnitude. Thinning a 20 ms run of a
switching converter by the same factor is the unsafe end: a switching overshoot is one or two
steps wide, so it falls between kept points and leaves no trace. When the feature is comparable
to the step, or you do not know, use `--format samples`.

### Why this is needed

The integration step and the rate you need to *look* at the answer can legitimately differ by
orders of magnitude. The case this was added for: verifying zero-voltage switching requires a
step below the hard discharge time of the switching node, `r_on * C_oss` — 0.4 ps at 1 mΩ and
400 pF. A larger step walks clean over the discharge, and soft and hard switching become
indistinguishable, which is a silent and very convincing failure.

A 150 ns commutation window at that step is 3.75 million points. At ~55 columns that is about
2.5 GB of CSV — for a measurement that needs a few thousand points. The run is affordable to
compute and not to write down:

```bash
general-simulator zvs.cir --mode transient --tfinal 150e-9 --dt 4e-14 --out-every 1000
```

3750 rows, 40 ps apart — still 2500× finer than the ~46 ns transition being resolved.

### What it is not

Not a substitute for a coarser `--dt`, and not adaptive stepping. Adaptive stepping would
lengthen the step exactly across the flat dead band where the discharge is about to happen,
which is the opposite of what a case like the above needs; `--out-every` keeps the guaranteed
uniform step and thins only the record of it.


## `--events`: asking for the event stream instead of finding it

The two event files default to the netlist's own directory, which is right when you run the
binary by hand and wrong for every tool. A front end compiles a schematic in memory and writes
the `.cir` into a temporary directory; both files were then created inside it and deleted with
it, so every event of every run was discarded — and nothing could have read them without
reconstructing the `<deck>.events.*` convention from the outside.

```bash
general-simulator buck.cir --mode transient --tfinal 2e-3 --dt 1e-8 \
  --events /tmp/run-17/events.jsonl --events-text /tmp/run-17/events.txt > /tmp/run-17/wave.csv
```

The two move independently on purpose: a tool reads the JSONL and a person reads the text, and a
caller that redirects one should not be handed the other somewhere it never looks.

A path whose directory does not exist is an error naming the path, and the run does not start.
Losing the diagnostics channel in silence is the failure these flags exist to fix, so it is not
something they will do themselves.

## `--format`: CSV, SPICE rawfile, or samples

`--format csv` (the default, and the only format this crate produced before `--format raw`
existed) prints `t,V(node1),V(node2),...,<block names>` to stdout, one row per resolved point —
see [Reading the output](reading-output.md) for the full column layout.

`--format raw` writes the *same data* — same columns, same values, same row order — as a binary
SPICE **rawfile** instead: the format `ngspice` and the wider SPICE-tooling ecosystem (including
Python readers such as PySpice and `spicelib`) read and write, for interoperability with tooling
that expects it rather than a CSV. Because a binary format has nowhere sensible to go on a
terminal, `--format raw` always writes to a file:

- `--out <path>` names it explicitly, or
- if `--out` is omitted, it defaults to `<netlist stem>.raw` next to the input file (e.g.
  `general-simulator buck.cir --mode transient --format raw` writes `buck.raw`).

`--format samples` writes the same data again, into a `.gsim_samples` container: each signal
stored separately, with a precomputed min/max pyramid over it, so a plot reads a screen's worth
of points rather than the whole run. It follows the same file rule as `--format raw` — `--out
<path>`, or `<netlist stem>.gsim_samples` beside the input. Reach for it when a run is large
enough that opening it is the problem; [Runs too large to open](large-runs.md) has the measured
numbers and says why `--out-every` is not the same thing.

`--out` given alongside `--format csv` is accepted but ignored — CSV output is always to stdout.

See [Reading the output](reading-output.md) for exactly what "same data" means (the variable
set/type mapping, and the `t` → `time` rename SPICE's own convention expects) and
`crates/general-simulator-cli/src/raw_format.rs`'s own module doc comment for the exact byte
layout, verified against the real format spec and cross-validated against two independent Python
readers (`tests/raw-output-python/`).

## `--mode dc` and block-driven gates

Every ideal switch gate in this crate is block-driven (`gate=block ctrl=<sig2phys-voltage-name>`, resolved
by reading the named block's current output each step — see the module doc comment on
`crates/general-simulator-cli/src/main.rs`). A `.op`-style DC operating point (`--mode dc`) has
no notion of a block's time-stepped state at all, so any netlist declaring an ideal switch is rejected
under `--mode dc` with a specific error naming the device, not run with some fixed/default gate
state — use `--mode transient` instead.

## Source material this was adapted from
- `crates/general-simulator-cli/src/main.rs`'s `usage()` function and its argument-parsing code.
- `crates/general-simulator-cli/src/raw_format.rs`'s module doc comment (rawfile format detail).
