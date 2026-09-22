# Checkpoint and resume

A transient run can save its complete state to a file and continue from it later —
in another invocation, on another machine, after the first run was killed — and the continued
run is **bit-identical** to one that never stopped. Not "close", not "within tolerance": every
number in the output is the same.

```bash
# Run the first 150 µs and save where it ended.
general-simulator buck.cir --mode transient --tfinal 1.5e-4 --dt 2e-7 \
    --checkpoint-out half.ckpt > first.csv

# Continue to 300 µs. --tfinal is absolute simulated time, not "another 150 µs".
general-simulator buck.cir --mode transient --tfinal 3e-4 --dt 2e-7 \
    --resume half.ckpt > second.csv
```

`first.csv` followed by `second.csv` (minus its header) is byte for byte the CSV of a single run
to `3e-4`. This is the property the feature is tested against, in fixed and adaptive stepping,
on a deck with a PWM-gated ideal switch, a freewheeling diode, `prev:` signals and every stateful
block kind — see `crates/dae-runtime/tests/checkpoint_resume.rs` and
`crates/general-simulator-cli/tests/checkpoint_cli.rs`.

## When it helps

- **Settling is the expensive part.** A converter whose output filter or control loop takes
  thousands of switching periods to reach steady state: reach it once, checkpoint, and ask every
  subsequent question (a load step, a different measurement window) from there. This is the
  companion of `ic=` ([Grammar overview](netlist-grammar.md#ic-initial-conditions),
  [Dynamic blocks](dynamic-blocks.md#ic-starting-a-block-somewhere-other-than-rest)): `ic=` is
  for an operating point you *know*; a checkpoint is for one you can only reach by simulating.
- **Long runs that might not finish.** `--checkpoint-every T` rewrites the checkpoint file every
  `T` seconds of simulated time (atomically — the previous good file is replaced, never left
  half-written), so a run killed by the operating system loses at most `T` of simulated time:

  ```bash
  general-simulator big.cir --mode transient --tfinal 0.5 \
      --checkpoint-out big.ckpt --checkpoint-every 1e-3 > big.csv
  # ... killed at t = 0.31 ...
  general-simulator big.cir --mode transient --tfinal 0.5 --resume big.ckpt > rest.csv
  ```

## What a checkpoint holds, and what it does not

Everything the transient loop carries from one accepted step to the next: the circuit state and
its one-step history (so trapezoidal integration continues rather than restarting with a
backward-Euler step), every diode's segment and every switch's gate state, the ringing-detection
cooldown, the adaptive controller's next step size, what every `prev:` signal will read, and
every block's own state — integrator values, sample-time accumulators and held outputs, VCO and
PWM phases, PMSM states, logic bits, counts. A `kind=pyblock`'s state object travels as
`pickle` bytes, with no change to the block's own code.

It does **not** hold the netlist. A checkpoint is loaded *into* the deck you pass, and it
refuses a deck that is not the one it came from — an edited value, a renamed node, an added
block — with `CheckpointDeckMismatch`, listing the unknowns on each side. Change the deck and you
start from `ic=` or rest, as before; that is deliberate, since a state vector has no meaning
against equations it was not solved from.

`ic=` values in the netlist are ignored on `--resume`: the precedence is `--resume`, then `ic=`,
then rest.

## Adaptive stepping: exact, with one honest caveat

A checkpoint written *mid-run* (`--checkpoint-every`) is taken after an ordinary accepted step,
and resuming from it reproduces the rest of the run exactly. A checkpoint written at `--tfinal`
sits on a step the controller *clamped* to land on `--tfinal` — a point an uninterrupted run
would not have stepped to. Resuming from it is still an exact continuation of the saved state
(the controller's own suggested step, not a restart from `--dt-init`), but from there the step
sequence is a different, equally valid one, so the two CSVs are not byte-identical the way they
are with `--dt`. If you need bit-identity across a split under adaptive stepping, split at a
periodic checkpoint.

## Escape-hatch blocks

| kind | checkpoints? | how |
|---|---|---|
| `pyblock` | yes | the state object is pickled; anything `copy.deepcopy` accepts almost always pickles too |
| `pyfunc`, `octfunc` | yes | stateless hosts — only their sample-time bookkeeping is stored |
| `octblock` | yes | the instance's own state slot is written with Octave's `save -binary` and read back with `load` — no change to your `.m` files |
| `cscript` | **opt-in** | the library exports `cscript_state_size`, `cscript_state_write` and `cscript_state_read` — see [the CScript chapter](cscript.md#checkpoint-and-resume) |

A `cscript` block's state is an opaque C heap object that only its own library can lay out,
so checkpointing it needs three more exported functions. A library exporting **none** of them
behaves as before: the run is fine without `--checkpoint-out`/`--resume`, and with one it is
refused at the first checkpoint with `CheckpointUnsupportedBlock`, naming the block — never
written partially, and no file is left behind:

```text
error: CheckpointUnsupportedBlock { block: "G1", kind: "cscript" }
```

A library exporting only **some** of the three does not load at all, whether or not a
checkpoint was asked for, with a `MissingSymbol` error naming the one that is absent.

`octblock` needs no opt-in: whatever `<function>_start` returned — a scalar, a struct, a
matrix — is what Octave's own binary format saves, and it stores doubles as their exact bits,
so the resumed run is bit-identical exactly as it is for the built-in blocks.

## The file

A small binary file: a `GSCK` magic, a format version, then the state in `postcard` encoding,
chosen because it round-trips every `f64` bit-exactly. It is not meant to be edited by hand and
is not stable across format versions — a file from another version is refused up front, not
misread. Size is a few hundred bytes to a few kilobytes for typical decks.
