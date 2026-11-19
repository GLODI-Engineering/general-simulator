# The output contract

A run produces four things: an **identity map**, a **status**, a **waveform**, and an **event
stream**. This chapter specifies all four, field by field, and says for each whether this engine
produces it today.

It exists because the displayer is not this repository. A schematic editor has to show what a run
found, and it must be able to show a run that `ngspice` produced, or Xyce, or whatever is written
next. That only works if the engine's output is a contract rather than "whatever the CSV parser
happened to accept". Writing it down is also what lets the two sides be built independently: a
front end implements *this*, not `general-simulator`.

The contract is deliberately small. Everything in it is something a displayer cannot work without,
and nothing in it describes how a display should look.

## The four layers

| Layer | Answers | Produced by |
|---|---|---|
| **L0 — Identity** | which drawn object is `Rload`? | the **compiler**, not the engine |
| **L1 — Status** | did the run finish, and is its result usable? | the engine's exit code and stderr |
| **L2 — Waveform** | what were the values over time? | the engine's result file |
| **L3 — Events** | what did the engine *notice*? | the engine's event stream |

L0 is listed first because the other three are useless without it, and because it is the layer
most easily forgotten: each of L1–L3 names things — `Rload`, `V(n_Rload_p2)`, `target: "DSW"` —
and a name is not a document object.

---

## L0 — Identity

**The engine does not know what the user drew.** It receives a netlist, in which every device is a
name. The map from those names back to the schematic belongs to whoever *generated* the netlist,
which for this project is `gsim-core`'s compiler, and it must travel with the run.

This is not a convenience. The compiler's naming function is not injective: a block id is prefixed
with its device letter only when it does not already start with it, so blocks `load` and `Rload`
both compile to the element name `Rload`. A front end cannot recover the map by reimplementing
that function, however faithfully — the information is gone. Two consequences, both now settled:

- the compiler refuses a name claimed twice (`gsim-core` `CompileError::DuplicateName`), and
  `general-mna` refuses a duplicate element name whatever the device letter (0.5.1 — before that,
  only the letters whose name becomes an unknown were checked, so two resistors of one name
  compiled, solved, and produced the right voltages in silence);
- the compiler **emits** the map, as `CompileOutput.provenance`, through both of its bindings.

### Shape

```json
{
  "elements": [
    {"element": "Rload",    "blocks": ["load"],      "role": "element"},
    {"element": "DSW",      "blocks": ["SW"],        "role": "element_and_block"},
    {"element": "FT1_pri",  "blocks": ["T1"],        "role": "part", "suffix": "pri"},
    {"element": "X2.Rin",   "blocks": ["X2", "in"],  "role": "element", "def": "LEAF"}
  ],
  "nets": [
    {"net": "n_Rbot_p1", "ports": [{"blocks": ["Rload"], "port": "p2"},
                                   {"blocks": ["Rbot"],  "port": "p1"}]}
  ]
}
```

| Field | Meaning |
|---|---|
| `element` | the name as the **flattened** deck spells it, which is what L2 and L3 carry: `X1.Rin`, not the body-local `Rin` |
| `blocks` | block ids from the top sheet inward; the last is the block that produced the card, the ones before it are the instances it sits inside |
| `role` | `element`, `block`, `element_and_block`, `part`, `instance` — what kind of card claimed the name |
| `suffix` | on a `part` only: which half of a composite (a transformer's `pri`/`sec`) |
| `def` | the `.SUBCKT` whose body it came from, absent at the top level |
| `nets[].ports` | every drawn port on that node, including a ground symbol's and a subsheet connector's |

### Rules

1. A name appears **at most once** in `elements`. A displayer may index by it.
2. `blocks` is never empty, and every id in it exists in the document the run came from.
3. Matching is **case-insensitive**: `general-mna` upper-cases element names internally, so a
   displayer comparing an event's `target` with an `element` must do the same.
4. An engine that is handed a netlist written by hand has no L0 at all. That is a legitimate run;
   a displayer must then show names and not pretend to know components.

**Status today:** produced by `gsim-core` ≥ the provenance change, through `gsim_py.compile` and
the wasm `compile`. **Not yet threaded** into the result document — see
[What does not exist yet](#what-does-not-exist-yet).

---

## L1 — Status

An exit code, and stderr for the human-readable part.

| Code | Meaning | Is there a result? |
|---|---|---|
| `0` | finished; nothing to report, or warnings only | yes |
| `1` | the run failed — bad arguments, an unreadable netlist, a solver error | no |
| `2` | finished, and a rule of severity `error` or `fatal` fired | **yes** |
| `3` | a rule ended the run early, cleanly: the current step finished, every file was flushed | **yes, up to the stop** |
| `4` | the solved state went non-finite and the run was abandoned | no |

The three distinctions that matter to a consumer:

- **2 and 3 are not failures.** They are completed runs whose results are exactly what the user
  needs to see, because something was found in them. A consumer that treats a non-zero code as a
  crash throws away the run that matters most.
- **3 is not 4.** A rule stopping a run is the deck behaving as written; a non-finite state is a
  run with no result at all. Reporting them the same way tells the user nothing.
- **1 keeps its old meaning.** It has always meant "this run failed" to every script and to the
  GUI's own solver discovery, so warnings did not take it.

### Rules

1. A displayer must classify on the code, not on the presence of output.
2. For `2` and `3` it must show the waveform **and** the events, with the run marked as having
   found something, not as broken.
3. For `3` it must say the run was stopped and by which rule — the event stream names it.
4. stderr is for a person. A displayer may show it verbatim; it must not parse it.

**Status today:** all five codes are implemented in the CLI (`mod exit`).

---

## L2 — Waveform

One named signal per column, all sharing one sweep axis.

| Field | Meaning | Required |
|---|---|---|
| `name` | the signal's name: `V(<node>)`, `I(<branch>)`, or a block's own output name | yes |
| `samples` | one value per point of the sweep axis, in axis order | yes |
| `unit` | the SI unit `samples` are in (`V`, `A`, `W`, `s`, ``) | yes |
| `quantity` | what kind of thing it is (`voltage`, `current`, `signal`) | no |
| `block` / `port` | the document object, resolved through L0 | no |

and one axis: `name` (`t`), `unit` (`s`), `samples`.

### Rules

1. **Values are exact.** CSV renders each `f64` with its own `Display`, which is the shortest
   decimal that round-trips, so a reader gets the solver's bits back. A displayer may narrow for
   rendering; it must not narrow before computing anything.
2. **Decimation is a count, not a resample.** `--out-every N` emits every Nth resolved point.
   Points are dropped, never averaged or interpolated, so a decimated waveform is a subset of the
   full one and a peak may be missing from it. A displayer that measures must say which it
   measured.
3. **Row order is axis order**, and the axis is monotonic.
4. A name is unique within one run.
5. The first column is the axis (`t` in CSV, `time` in a rawfile — SPICE's own name for it).

### Rules for the identity link

A signal's `block`/`port` is **derived**, never guessed:

- `V(<net>)` resolves through `nets` to every port on that node.
- `I(<element>)` and a block output resolve through `elements` to one block path.
- Anything that resolves to nothing stays a bare name. A placeholder is worse than a blank: it
  looks like an answer.

**Status today:** CSV (default) and SPICE rawfile (`--format raw`) both carry name and samples,
built from the same in-memory data, directly comparable row for row. **Neither carries a unit**,
and no writer fills `block`/`port`. The GUI's `.gsim_sim_result` v2 document has all four fields
(`signals[].unit`, `.quantity`, `.block`, `.port`) and is where they belong.

---

## L3 — Events

What the engine noticed, as a stream. One record shape feeds the JSONL log, the console line, the
summary table and (eventually) the viewer's markers, so those four can never disagree about what
happened.

### The event record

```json
{"record": "event", "time": 4.9992e-4, "step": 4999, "step_time": 5.0e-4, "rule": "deadtime",
 "target": "lega",
 "kind": "instant", "severity": "warning", "action": "console", "value": 0.0, "limit": 1.0e-7,
 "unit": "s", "detail": "lega_high, lega_low"}
```

| Field | Meaning | Required |
|---|---|---|
| `time` | simulation time, **interpolated** to where the quantity crossed the limit inside the step | yes |
| `step` | the solver step the crossing was noticed in | yes |
| `step_time` | the solver's own time at the end of that step, before interpolation | yes |
| `rule` | the rule's name, as the deck spells it | yes |
| `target` | the component, switch or signal watched — an **L0 name** | yes |
| `record` | `event` — what shape this line is, so a reader never discriminates on a missing field | yes |
| `kind` | `enter`, `leave`, `instant` | yes |
| `severity` | `info`, `warning`, `error`, `fatal` | yes |
| `action` | what was actually **done**, which is not always what the rule asked for | yes |
| `value`, `limit` | what the rule compared | yes |
| `unit` | the unit `value` and `limit` are in | no (empty = dimensionless) |
| `duration`, `peak`, `peak_time` | the interval that just ended — on a `leave` only | no |
| `truncated` | set when the run ended with the condition still true, so `duration` is a lower bound | no |
| `detail` | which components are responsible, when the rule spans several | no |

### Rules

1. **`time` is not `step_time`.** The interpolated time is the answer; the step time is there so a
   reader can tell a crossing placed inside a step from one that landed on a boundary, and so the
   record can be lined up against a waveform whose rows sit on those boundaries. A displayer
   puts the marker at `time` and seeks to `step`.
2. **A violation is two records**, `enter` then `leave`, and the second carries the measurement.
   An `enter` has no duration because it is not known yet; that is the honest encoding, which is
   why a leave is a separate record rather than a mutation.
3. **`severity` and `action` are independent.** The same over-current is a `warning` someone reads
   afterwards and a `fatal` that stops a regression run; the rule should not have to be rewritten
   between the two. A displayer colours on `severity` and explains on `action`.
4. **`action` is what happened.** A `pause` in a batch run is recorded as the `console` it became.
   A displayer must not infer the configured action from the record.
5. `unit` travels on the record, not with whoever prints it: one rule can report in two units — a
   leg rule's violation is a count of switches and its dead-time finding is a time. Printing the
   second in the first's unit produced `0.0000 conducting`, which is how the field came to exist.
6. **Every rule reports, including the quiet ones.** A rule that saw nothing, a rule whose
   quantity was missing from the sample, and a rule whose findings a budget suppressed are three
   different states and none of them is "passed".

### The per-rule summary

One record per rule at the end of the run, tagged `"record": "summary"`: `entries`, `filtered` (excursions a hold time
discarded), `suppressed` (records a `max_events` budget kept out of the log), `total_time`,
`worst_value`, `worst_time`, `missing_samples`, `instants`, `shortest_deadtime`.

`missing_samples` above zero means the rule was partly or wholly **blind**, which a displayer must
not render as a pass. `filtered` above zero means the hold time is hiding something.

**Status today:** written to `<netlist stem>.events.jsonl` (one JSON object per line) and
`<netlist stem>.events.txt` — or wherever `--events <path>` / `--events-text <path>` ask for them,
independently — plus the console for anything whose action earns it. Both shapes are tagged:
every line carries `record`, `"event"` or `"summary"`, as its first field.

---

## Versioning

1. The result document carries `schema_version`. A displayer refuses a version it does not know,
   by number, rather than guessing from the fields present.
2. **Adding an optional field is not a version change.** A consumer ignores fields it does not
   know, and therefore an older displayer keeps working against a newer engine.
3. Removing a field, renaming one, or changing a unit **is** a version change.
4. A field's absence has one meaning: *this engine does not produce it*. It never means zero, and
   never means false.

## What an engine must provide to join

This section describes what **any** engine would have to provide to join the contract, in the
order it would have to provide it. It is not a roadmap: no second engine is in progress here, and
the engine side of this project has one implementation. `ngspice` is used as the example because
it is the most likely first candidate from the editor's side, where multiple engines are the
stated direction.

A new engine — `ngspice` as the example — needs, in this order:

1. **L1**: a documented mapping from its own exit status onto the five codes above. Anything else
   makes "it worked" unanswerable.
2. **L2**: names, samples, one monotonic axis, and units. The rawfile reader already in this
   repository covers `ngspice`'s native output; units come from the rawfile's own variable types.
3. **L0**: whatever its netlist generator knows. For a deck this project compiled, the compiler's
   provenance map applies unchanged, because it describes the *document*, not the engine.
4. **L3**: optional. An engine with no event stream is a usable engine with an empty event list —
   not a broken one. A displayer must render that case without a placeholder that looks like a
   pass.

## What does not exist yet

Each of these is a gap in the contract's implementation, not in the contract. They are listed here
because specifying them as though they were done is how a contract becomes fiction.

- **A solver failure is not an event.** A Lemke `RayTermination` carries no time, no step and no
  switch state, so there is nothing to put a marker on and nothing to tell the user which switch
  configuration failed to solve. L1 reports the run died; L3 cannot say where.
- **`pause` is accepted and then downgraded.** `Action::Pause` is recorded and treated as
  `Console`, because a command-line run has nobody to resume it. Holding at an event with the
  state intact is Phase 5.
- **L0 is not threaded through.** The compiler emits the map; nothing carries it into the result
  document, so `signals[].block`/`.port` are unfilled.
- **L2 carries no unit.** Neither CSV nor the rawfile writer emits one, though the result document
  has the field.

## Evidence: what the current adapter loses

The three claims above are measured, not predicted. Omni Schematics' existing adapter
(`omni-schematics`, its own simulator-solver layer):

- writes the netlist into a `tempfile.TemporaryDirectory()`, so `<stem>.events.jsonl` and
  `<stem>.events.txt` are **deleted with it** — every event of every run is discarded;
- raises on any non-zero exit code, so a run that a rule stopped (`3`) or in which a rule fired
  (`2`) is shown as a crash with no waveform;
- fills `Signal(block_id="general_simulator", port_id=f"signal_{index}")` — a placeholder in
  fields the result format defines, because nothing told it which block `Rload` is.

None of the three is a bug in that adapter. All three are what a missing contract costs.
