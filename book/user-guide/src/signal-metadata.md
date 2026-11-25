# What a signal is, beside its name

A run's columns are not all the same kind of thing. `V(out)` is a potential in volts, `I(L1)` is
a current in amperes, and `DUTY` is a fraction of a switching period that is not measured in
anything at all. A consumer that has only the names has to guess which is which, and the guess
everyone reaches for — `V(` is volts, `I(` is amperes — is wrong in exactly the case this project
cares about most, because a signal-flow value printed as `0.42` looks like `0.42` volts to
anything that labels an axis from a prefix.

This chapter is the convention that fixes that: how a quantity's nature travels with it, so that
this simulator's container writer, a waveform viewer, a Python script reading a CSV header and a
`kind=measure` statement all answer the question the same way instead of each inventing a rule.

## The five rules

**1. A quantity's nature travels *beside* its name, never inside it.**

A column's name is an interface. `kind=measure` names columns (`out=DUTY`), scripts name columns,
and a CSV header is something people open in a spreadsheet. Encoding a unit into the name — a
suffix, a prefix, a namespace — would travel through every format for free and break every one of
those at the same time. So nothing is encoded: `V(out)` stays `V(out)`, byte for byte, and the
metadata rides alongside.

**2. Two fields, two different questions.**

`quantity` says what *kind* of thing a column holds. `unit` says what it is measured in. They are
not the same question, and collapsing them is what makes a dimensionless value indistinguishable
from an unknown one.

**3. The vocabulary is closed and small.**

`quantity` is one of `time`, `voltage`, `current`, `signal`. Four entries, because a vocabulary a
consumer cannot enumerate is no better than free text. A reader that meets a `quantity` it does
not recognise should treat it exactly as it treats an empty one — as *not known* — so the list can
grow later without breaking anything already written.

**4. An empty `unit` means "not known". It never means "dimensionless".**

A dimensionless value says so through `quantity`, not by leaving `unit` blank. This is the rule
that makes the whole thing useful: `quantity=signal` with no unit is a *positive statement* —
this is a signal-flow value, not a potential — rather than a shrug. A writer that cannot be sure
of a unit leaves it empty rather than offering a plausible one, because a plausible wrong unit is
worse than an absent one: nobody checks the one that looks right.

**5. Nothing is inferred from how a name is spelled.**

What a column is comes from what it is *in the run*. The sweep axis is the sweep axis. An unknown
of the circuit's MNA system is a node potential or a branch current, and `general-mna` writes
which one it generated into the name (`V(...)` for each node, then `I(...)` for each branch), so
reading that is reading a name this toolchain wrote to a fixed shape — not guessing at a name a
user chose. A column is never classified by its prefix before its position has already
established that it is a circuit unknown at all.

## What each column reports

| Column | `quantity` | `unit` |
|---|---|---|
| the sweep axis, `t` | `time` | `s` |
| a node voltage, `V(node)` | `voltage` | `V` |
| a branch current, `I(branch)` | `current` | `A` |
| `kind=phys2sig node=<n>` | `voltage` | `V` |
| `kind=phys2sig branch=<b>` | `current` | `A` |
| `kind=sig2phys domain=voltage` | `voltage` | `V` |
| `kind=sig2phys domain=current` | `current` | `A` |
| every other block output | `signal` | *(empty)* |
| one element of a vector signal, `NAME[i]` | whatever `NAME` is | likewise |

`analysis` carries which analysis produced the run — `transient` or `dc` — on every vector of it.

The two converter kinds are the only block outputs with a unit, and they have one for a concrete
reason rather than as a special case: `phys2sig` *names* the node or branch it reads, and
`sig2phys` *declares* the domain it drives, so in both cases the netlist itself has already said
what the value is. Every other block's output is a signal-flow value whose unit the netlist never
states. A `kind=pid`'s output may be a voltage command, a duty cycle or a modulation index; a
`kind=vco`'s is a phase; a `kind=cscript`'s is whatever its author decided. Guessing between
those is precisely the thing rule 4 forbids.

## Where to read it

**In a `.gsim_samples` container**, from the footer's `VectorMeta` — one per signal plus one for
the sweep axis, with `name`, `quantity`, `unit` and `analysis`. See
[Runs too large to open](large-runs.md) for the container itself. This is the format designed to
carry it, and it is where a viewer should look.

**In a CSV**, nowhere — and deliberately. The CSV is unchanged: one header line of names, then
rows of numbers, exactly as before. A second header line carrying units would be read as data by
every plain CSV reader that exists, including this project's own tests, so the format that is
valued for being trivially parseable stays trivially parseable. A tool that needs units from a
CSV run should ask for `--format samples` instead, or carry its own mapping; what it must *not*
do is derive them from the names, because rule 5 is a rule about consumers too.

**In a SPICE rawfile**, approximately. The rawfile's `Variables:` lines have a type field, but the
format has no generic-signal type and the readers in the wild reject an invented one, so a
signal-flow column is written there as `voltage`. That is the one place this convention is
knowingly lossy, and it is lossy because an external format forces it; the `.gsim_samples` footer
for the same run still says `signal`. See [Reading the output](reading-output.md) for the rawfile.

## What a netlist's producer has to do

The convention above is complete for a netlist written in this grammar, because this grammar
already keeps the two domains apart: a circuit quantity is a node or a branch, a signal-flow value
is a `kind=` block's output under its own name, and the only ways across are `phys2sig` and
`sig2phys`. The solver reports what the netlist says, and for a netlist written this way what it
says is right.

A netlist generated from a drawn schematic can lose that. If a generator emits a signal-flow wire
— a PID output, a duty command, a PLL angle — as an ordinary *node*, then that node's voltage is
reported as `voltage`/`V`, and correctly so: in this grammar a node is a potential, and a
generator that uses one to carry something else has not written down the one fact only it knows.
The solver cannot recover it, and will not invent a hedge for it.

So for a generated netlist the convention has a requirement on the generator, and it is a
requirement to use the grammar rather than to add anything to it — tracked for this toolchain's
own compiler as `GLODI-Engineering/gsim-core#77`:

- **Emit a signal-flow wire as a block output, not as a node.** Its nature then travels for free:
  every block output already reports `signal`, and a consumer can tell it from a voltage with no
  second map. This is what the grammar is for.
- **Emit a crossing as the crossing it is.** Where a signal-flow value has to become a circuit
  quantity, it goes through `sig2phys domain=voltage|current`, and the node it then drives
  genuinely *is* a voltage — so reporting `voltage` for it is a fact, not a guess. The same holds
  in the other direction for `phys2sig`.
- **If some construct genuinely cannot be expressed either way**, that is a gap in the grammar and
  needs a way for the netlist to mark a node's domain — a change to propose on its own, with its
  own discussion of what the netlist carries and what reads it. It is not something the solver
  should paper over by refusing to label any node at all, which would cost every honest netlist
  its units to protect a dishonest one.
