# gsim-rules

Rules a simulation checks while it runs.

A control or modelling mistake rarely announces itself. It produces a waveform that looks
plausible and is wrong: a transformer wired with the wrong sign absorbs power on both sides while
its primary trace looks correct, a magnetising current drifts from 40 A to 245 A with nothing to
show it, a half-bridge leg conducts through both switches and the ideal model answers with a large
current rather than the destroyed hardware it stands for.

This crate is how a run says so instead.

```rust
use std::collections::BTreeMap;
use gsim_rules::{Action, Condition, Rule, RuleEngine, Severity};

let rule = Rule::new(
    "i_max", "L1", "i(L1)",
    Condition::Above { limit: 50.0 },
    Severity::Warning,
    Action::Log,
);
let mut engine = RuleEngine::new(vec![rule], false);

let mut sample = BTreeMap::from([("i(L1)".to_string(), 10.0)]);
assert!(engine.observe(0, 0.0, &sample).is_empty());

sample.insert("i(L1)".to_string(), 60.0);
assert_eq!(engine.observe(1, 1e-6, &sample).len(), 1); // one enter, not one line per step
```

## What it reports, and what it refuses to

- **Edges, not samples.** One record when a violation starts, one when it ends, carrying the
  duration, the worst value and when it happened. A violation lasting 3 ms at 10 ns steps is two
  lines, not 300 000.
- **Sub-step times.** The time reported is where the quantity crossed the limit between two
  samples. At a 10 ns step every event would otherwise be quantised to 10 ns, which is the same
  order as the dead times these rules exist to measure.
- **A rule that could not read its quantity is not reported as passing.** Silence is ambiguous,
  and the ambiguous reading is the dangerous one, so a missing quantity is counted and surfaced.
- **It never touches the circuit.** Nothing here can change a value the solver computed. The one
  effect it has on a run is `Action::Stop` ending it. That is why it is its own crate and depends
  on no solver: it reads named quantities through the `Sample` trait and is tested against a plain
  map.

## Rules that span several components

A leg conducting through both switches is "how many are on, against how many may be" — a count,
with the same edges, hold time and duration as an over-current. So `Watch` reduces every kind to a
value and a margin and one engine serves them all, which is what makes the next rule type a
variant rather than a state machine:

- `Watch::Quantity` — one named scalar, judged by a `Condition`.
- `Watch::ForbiddenState` — switches that must never conduct together, with the dead time measured
  from the real transitions. A handover straight from one switch to the other, with no step in
  between where the leg was open, is a dead time of zero and is reported as one.
- `Watch::RequiredPath` — switches of which at least one must conduct while a current is flowing,
  for the opposite mistake: opening every switch in an inductor's path gives the current nowhere to
  go, and the solver answers with a voltage spike rather than an error.

## Licence

AGPL-3.0-or-later.
