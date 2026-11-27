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
D1 kind=ideal_diode g_breakdown=0 v_breakdown=-100 g_off=0 v_f=0.7 g_on=1
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

## Number suffixes

A value may be written plainly (`1000`), in exponent form (`1e3`), or with a SPICE
suffix (`1k`). Suffixes are **case-insensitive**, which produces the one trap in
this grammar that costs real time:

| Suffix | Multiplier | Written |
|---|---|---|
| `T` | $10^{12}$ | `1T` |
| `G` | $10^{9}$ | `1G` |
| `MEG` | $10^{6}$ | `1MEG`, `1meg` |
| `K` | $10^{3}$ | `1k`, `1K` |
| `M` | $10^{-3}$ | `1m`, **`1M`** |
| `U` | $10^{-6}$ | `1u`, `1U` |
| `N` | $10^{-9}$ | `1n` |
| `P` | $10^{-12}$ | `1p` |
| `F` | $10^{-15}$ | `1f` |

**`M` is milli, not mega.** `R1 in out 1M` is a one-milliohm resistor. Mega is the
three-letter `MEG`, and because the parser is case-insensitive there is no spelling
of a single `M` that means $10^{6}$:

```
R1 in out 1M        * 0.001 ohm
R1 in out 1MEG      * 1000000 ohm
```

This is SPICE's rule rather than ours, and every SPICE-family tool shares it — but
`M` is the obvious abbreviation for mega everywhere else an engineer works, and
nothing in a netlist flags the difference. A deck written with `1M` for a megohm
simulates happily, a billion times away from the circuit that was meant, and the
symptom looks like a wrong answer rather than a wrong input. See
[GOTCHA-003](https://github.com/GLODI-Engineering/general-simulator/blob/main/docs/gotchas/GOTCHA-003-capital-m-is-milli-not-mega.md)
for the measured demonstration.

A trailing unit is ignored, so `1kOhm`, `1kohm` and `1k` are the same value, as are
`10uF` and `10u`. The unit is for the reader; the parser stops at the suffix.

**One grammar, everywhere.** A device value, a `kind=` block field, a `kind=rule` field and a
`kind=measure` field are all read this way: `PWM1 kind=pwm freq=100k in=DUTY red=300n` and
`M kind=measure type=rms out=S from=200u to=500u` mean what they look like. Until
[#57](https://github.com/GLODI-Engineering/general-simulator/issues/57) the block and measure
fields went through a plain floating-point parse instead, so `freq=100000` was accepted on the
same line where `freq=100k` was refused with "is not a number", about a number.

A suffixed value is also the *same* `f64` as its exponent spelling, not a rounding of it: the
suffix is appended as an exponent and the literal parsed once, so `300n` and `300e-9` are one
value rather than two that differ in the last bit.

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

### Formal parameters

A `.subckt` may declare parameters with defaults after its port list, and an `X` call may give
each instance its own value — which is what makes one definition serve as a *component* rather
than a one-off group:

```spice
.subckt RPART p n Rv=1000
R p n {Rv}
.ends RPART

V1 A 0 5
X1 A MID RPART Rv=100
X2 MID 0 RPART Rv=220
```

`X1` is 100 Ω and `X2` is 220 Ω in the same solve. An instance that names no value takes the
definition's declared default, so `X3 A 0 RPART` is 1000 Ω. `PARAMS:` before the assignments is
accepted on both the definition and the call, for decks written for tools that require it.

Four rules, each of which can be read off the deck:

- **A value is substituted into the expression segments** — `{...}` and `'...'` — of every
  device card, block field and `.IC` value in the body. A bare token is left alone, because a
  bare token in a value position can be a model name or a device keyword, and the grammar
  already provides braces for an expression. A `.model` card inside a body is not substituted
  into; a model's name is global once the hierarchy is flat, so a parameterised one would mean
  two instances declaring the same model differently.
- **A body sees its own parameters and whatever is global, never its caller's.** A definition
  therefore means the same thing wherever it is called from. An expression inside a definition
  that names one of *its caller's* parameters is not resolved, and fails as the undeclared
  symbol it is.
- **A value written at a call site is read in the caller's scope**, so
  `X1 p n inner Ri={Ro}` inside a definition whose own parameter is `Ro` passes `Ro`'s value
  down. Nesting composes the same way the dotted node path does.
- **A `.param` inside a body may not depend on a parameter.** A `.param`'s name is global once
  the hierarchy is flat, so two instances would both declare it and one of them would lose;
  that case is refused. A `.param` that depends on nothing instance-specific is fine.

Three things stop the run rather than being quietly repaired, because each one would otherwise
mean solving a different circuit than the deck describes:

| deck | refusal |
|---|---|
| `X1 A MID RPART Rw=100` — a name the definition does not declare | names the instance, the parameter, the definition, and what it does declare |
| `X1 A MID RPART Rv=100 Rv=220` | the same parameter set twice has no defensible reading |
| a declared parameter with neither a default nor a value | named at the instance that owes it one, rather than much later as an undeclared symbol |

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
