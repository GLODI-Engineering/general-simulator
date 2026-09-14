# Grammar overview

[Your first netlist](first-netlist.md) already showed the core convention: a `kind=...` line
declares a PWL device model or a signal-domain block, and it can live either as a real
first-class statement or disguised as a `*`-prefixed SPICE comment so an un-migrated tool still
opens the file cleanly. This chapter goes one level deeper — the parts of the grammar that apply
across every `kind=...` declaration, not just one family of them — and then points into the
detail chapters for everything else.

## `kind=...` lines, real or disguised

`general-mna`'s own grammar (documented in full in `general-spice-core`'s `docs/GRAMMAR.md` §12)
gives `name kind=<kind> field=value ...` genuine first-class syntax — no `*`-comment trick
needed:

```text
D1 kind=ideal_diode g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.7 g_on=1
ONVAL kind=const value=1
ONGATE kind=sig2phys domain=voltage in=ONVAL
```

The older convention — the same lines, each prefixed with `*` so a real SPICE tool (or an
un-migrated netlist) sees an ordinary comment — still works unchanged: the lexer strips
`*`-prefixed lines to nothing before parsing either way, so a mix of old- and new-style lines in
the same file is fine (`crates/general-simulator-cli/src/main.rs`'s module doc comment). A line
without `kind=` at all is left alone, including a genuine comment that happens to start with
`*` — there's no ambiguity to worry about.

## `--devices <file>`

`--devices <file>` remains available for sharing one controller/PWL-parameter file across
several netlists, but it is not the default or the expected common case — the one-file
convention from [Your first netlist](first-netlist.md) is. **A `--devices` file is parsed with
the exact same grammar as the netlist itself** (`general_mna::build_system`, the same
`Dialect::Ngspice` parser either way — `crates/general-simulator-cli/src/main.rs`'s own
`devices_source` handling) — there is no separate device-file-only comment syntax: `*` in
column 1 is still a full-line comment, and `;`/`$`/`//` still end a line as a comment
mid-statement, exactly as in the main netlist. (An earlier draft of this chapter claimed `#`/`;`
as a distinct device-file comment convention; nothing in the current parser supports that — the
grammar genuinely doesn't distinguish the two files at all.)

## Blocks may reference each other in any order

Blocks are *not* evaluated in file order. Each step's actual evaluation order is derived
automatically from the `in=`/`inputs=` dependency graph itself
(`dae_runtime::block_graph::topological_order`), so a block may name another declared anywhere
in the file, before or after it. Declaring sources before sinks, left to right like a
signal-flow diagram, remains good practice for a human reading the file — it's just no longer a
correctness requirement. A genuine same-step cycle is rejected before any step runs; see
[Signals](signals.md) for the exact error and the `prev:` fix.

## The shared `r_on` constraint

Every ideal switch declared in one run must share the same `r_on` — `dae-runtime`'s switch
mechanism uses one shared on-resistance per call
(`dae_runtime::solve_dc_with_ideal_switches`'s doc comment). If two `kind=ideal_switch` lines
give different `r_on` values, that's not a netlist typo being silently tolerated; it's a real
limitation of the current solver worth knowing about before it surprises you.

## `ic=` initial conditions

A storage element may declare the state a transient run starts in, directly on its own `C`/`L`
card:

```text
C1 b 0 1e-6 ic=5      * 5 V across the capacitor at t = 0
L1 a b 5e-6 ic=12     * 12 A through the inductor at t = 0
```

**Sign conventions** (`general-mna/README.md`'s "Initial conditions" section, `MnaSystem`'s own
contract):

- **A capacitor's `ic` is the first node's voltage minus the second's.** `C1 b 0 1e-6 ic=5`
  means $V(b) - V(0) = 5\text{ V}$; writing `C1 0 b 1e-6 ic=5` declares $-5\text{ V}$ on node `b`
  instead.
- **An inductor's `ic` is the current flowing from its first node to its second, through the
  inductor** — exactly the sign of the system's own `I(<name>)` unknown, since a branch device's
  incidence stamp puts `+1` in its first node's KCL row. `L1 a b 5e-6 ic=12` puts 12 A into
  terminal `a` and out of terminal `b`; writing `L1 b a 5e-6 ic=12` declares the *opposite*
  physical current — the mistake worth checking for first when an inductor starts a run
  backwards.

**It is an assignment, not a solve.** The operating point is skipped: the declared states are
written directly into the starting state vector, every other unknown starts at rest (`0`), and
the system that actually gets solved is untouched — no element is swapped for a source, nothing
is opened or shorted. The result need not satisfy the circuit's own algebraic constraints at
`t=0` — that's not a defect, a first backward-Euler step re-imposes them, and it's the visible
difference from a constrained operating-point solve (which would let the rest of the circuit
move to accommodate a declared value). See [Fixed vs. adaptive time stepping](time-stepping.md)
for how this interacts with the first step of a run.

Contradictions are reported rather than silently resolved one way or the other
(`MnaSystem::initial_state`, via `InitialStateError`):

- `ic=` values that contradict each other (a loop of `ic`-bearing capacitors whose declared
  voltages don't sum to zero) — `InitialStateError::ConflictingConditions`.
- An assignment that violates one of the circuit's own algebraic equations once every unknown in
  it is already fixed by an `ic=` (an `ic` on a capacitor wired directly across an ideal voltage
  source, or two series inductors whose `ic`s declare different currents) —
  `InitialStateError::InconsistentWithCircuit`.

## `.subckt`/`X`-instance hierarchy

A `.subckt`/`X`-instance pair works the same way it does in any real SPICE tool, expanded into
one flat statement list before either the electrical or the block-graph builder ever sees it
(`general-mna/src/hierarchy.rs`). Every device and block inside an instance is renamed with a
dotted path built from its instantiation chain — `X1.R1`, `X1.X2.C3` — so repeated instances of
the same subcircuit never collide, and the hierarchy stays visible everywhere downstream: gate
bindings, `phys2sig` block names, CSV column headers.

A `.subckt`'s declared port list isn't restricted to electrical nodes — nothing in the grammar
distinguishes an electrical port from a signal-domain one, so the same name-substitution
machinery that rewires an internal node to whatever the `X`-call site bound it to also rewires a
block's `in=`/`inputs=` field or its own `.name`:

- **Signal in**: `.subckt reg vin vout ctrl` with an internal block field `in=ctrl` — calling
  `X1 vsrc vload EXT_DUTY reg` binds `ctrl` to `EXT_DUTY`, so the internal block reads whatever
  `EXT_DUTY` (declared outside the subckt, at whatever scope `X1` is itself called from)
  produces.
- **Signal out**: an internal block whose own `.name` *is* the declared port name (e.g. a
  `kind=phys2sig` block literally named `reading` inside `.subckt sensor vin vout reading`)
  becomes addressable from outside under whatever name the caller bound that port to (`X1 a b
  MEASURED sensor` exposes it as `MEASURED`) — symmetric with how an internal node sharing a
  port's name is externally addressable through the caller's own binding.

A block reference that resolves to nothing declared (not a port, not another block in the same
body) still surfaces as the ordinary `UnknownBlockInput` error, just after expansion instead of
before.

## Device-card parameter checking

Every trailing token on a device card — positional and `key=value` alike — is checked against
what that device letter's own grammar actually permits, not silently dropped
(`general-mna/README.md`'s "Device-card parameter checking"):

| letter | positional parameters accepted |
|---|---|
| `R`, `C`, `L` | exactly one value |
| `K` | exactly one value, the coupling coefficient |
| `E`, `G` (classic four-node form) | exactly one value, the gain |
| `F`, `H` | exactly two: a controlling source's name, then a gain |
| `V`, `I` | a bare leading DC value and/or `DC <value>`, an `AC <magnitude> [<phase>]` spec, and at most one `SIN`/`PULSE`/`EXP`/`PWL`/`SFFM` call |
| `D` | a model name, an optional area factor, an optional `OFF` |

`key=value` fields are checked against the short list each letter accepts: `ic` on `C`/`L`,
nothing anywhere else. `R1 a 0 1000 tc1=0.001` and `C1 b 0 1e-6 wibble=5` are both build errors
naming the line and the device — a stray or misspelled field doesn't silently vanish and leave a
plausible-looking waveform for a circuit nobody actually wrote.

**Known limits of that checking**, worth knowing about rather than assuming total coverage:

- **A diode's model name isn't checked at all.** This crate never stamps a diode from a device
  model — a `D` card becomes a symbolic conductance/current-source pair, with the actual
  `kind=ideal_diode`/`kind=ideal_switch` line supplying the real numeric physics (see
  [PWL devices](pwl-devices.md)) — so there's no parameter set to check the model name against,
  and even a crate that had one couldn't settle the question from the netlist alone (the real
  `.model` card, in a genuine SPICE deck, may live behind an `.include` this crate never
  resolves).
- **A controlled source's controlling-source name** is checked, but later, by a different error
  (`UnknownControllingBranch`) once the branch index exists.
- **A symbol where an `AC` magnitude or phase belongs** — `AC {gain}` is a legitimate
  parameterized magnitude and nothing in the token distinguishes it from a misspelling, so at
  most two value-shaped tokens are consumed after `AC`; a third falls through and is rejected.
- **Every parameter of a device letter this crate has no stamp for at all** — `M`, `Q`, `J`, and
  the rest — is ungoverned by this checking; those letters are handled by a separate
  unsupported-element policy instead.
- **A switch-assigned element's positional parameters** aren't checked — its value is replaced
  wholesale by the configured on/off resistance, so its own grammar isn't this crate's to
  enforce (its `key=value` fields still are).

## Where the rest of the grammar lives

This chapter stays deliberately short — it's the index into the detail chapters, not a
restatement of them:

- [PWL devices: diodes and ideal switches](pwl-devices.md) — `kind=ideal_diode`/
  `kind=ideal_switch` field reference and the node-order convention.
- [Gate bindings](gate-bindings.md) — how an ideal switch's gate state is actually resolved.
- [Signals](signals.md) — `prev:`, same-step references, and the `AlgebraicLoop` error.
- [Block library overview](block-library.md) and the [Component Reference](component-reference.md)
  — every `kind=...` block's own full field list.
