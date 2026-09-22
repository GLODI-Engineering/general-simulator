# Command-line flags

```text
general-simulator <netlist> [--devices <file>] [--mode dc|transient] [--tfinal T]
                   [--dt DT | --dt-max T --dt-min T --dt-init T --reltol R --abstol A]
                   [--format csv|raw] [--out <path>] [--out-every N]
                   [--checkpoint-out <file>] [--checkpoint-every T] [--resume <file>]
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
| `--format csv\|raw` | Output serialization — see below. Default: `csv`. |
| `--out <path>` | Destination file for `--format raw`. Ignored for `--format csv` (CSV always goes to stdout). |
| `--out-every N` | Write only every Nth resolved point. Thins the output, never the computation — see below. Default: `1`. |
| `--checkpoint-out <file>` | Write the run's complete state at `--tfinal` so a later run can `--resume` it. See [Checkpoint and resume](checkpoint-resume.md). |
| `--checkpoint-every T` | With `--checkpoint-out`: also rewrite that file every `T` seconds of simulated time, so a killed run loses at most `T`. |
| `--resume <file>` | Continue from a checkpoint instead of from $t = 0$; `--tfinal` stays absolute. Only loads into the deck it was written from. |

`--dt` and any of `--dt-max`/`--dt-min`/`--dt-init`/`--reltol`/`--abstol` are mutually
exclusive — pick fixed-step or adaptive stepping, not both. Omitting `--dt` entirely selects
adaptive stepping, the default, the same local-truncation-error approach every real SPICE-family
tool uses by default when only the run length is given.

## `--out-every`: when the timestep and the useful output rate differ

`--out-every N` writes every Nth resolved point and skips the rest. It affects **serialization
only**:

- every step is still taken, so the solution is bit-identical to the same run without the flag;
- `kind=measure` still sees every point, so measurement results are unchanged (decimating a
  measurement's input would change its *answer*, not just its size);
- `--out-every 1`, the default, is byte-for-byte the behaviour from before the flag existed.

Point 0 is always written. After that it is simply every Nth, so the final point appears only if
its index lands on the stride.

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

## `--format`: CSV vs. SPICE rawfile output

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
