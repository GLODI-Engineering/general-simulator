//! Rules a simulation checks while it runs.
//!
//! A control or modelling mistake rarely announces itself. It produces a waveform that looks
//! plausible and is wrong: a transformer wired with the wrong sign absorbs power on both sides
//! while its primary trace looks correct, a magnetising current drifts from 40 A to 245 A with
//! nothing to show it, a leg conducts through both switches and the ideal model answers with a
//! large current rather than the destroyed hardware it stands for. This crate is how a run says
//! so instead.
//!
//! A [`Rule`] watches something, a [`Condition`] decides when that counts as a violation,
//! [`Qualifiers`] decide when a true condition is worth reporting, and a [`Severity`] with an
//! [`Action`] decides what happens. The [`RuleEngine`] turns a stream of samples into
//! [`EventRecord`]s on the **edges** of each violation -- one when it starts, one when it ends
//! -- so a violation lasting 3 ms at 10 ns steps is two lines rather than 300 000.
//!
//! # What this crate deliberately does not do
//!
//! It never touches the circuit. Nothing here can change a value the solver computed; the one
//! effect it has on a run is [`Action::Stop`] ending it. That is why it is a separate crate
//! from `dae-runtime` rather than a module inside it, and why it depends on no solver at all:
//! it reads named quantities through the [`Sample`] trait, so it builds and is tested against a
//! plain map.
//!
//! ```
//! use std::collections::BTreeMap;
//! use gsim_rules::{Action, Condition, Rule, RuleEngine, Severity};
//!
//! let rule = Rule::new(
//!     "i_max", "L1", "i(L1)",
//!     Condition::Above { limit: 50.0 },
//!     Severity::Warning,
//!     Action::Log,
//! );
//! let mut engine = RuleEngine::new(vec![rule], false);
//!
//! let mut sample = BTreeMap::from([("i(L1)".to_string(), 10.0)]);
//! assert!(engine.observe(0, 0.0, &sample).is_empty());
//!
//! sample.insert("i(L1)".to_string(), 60.0);
//! let events = engine.observe(1, 1e-6, &sample);
//! assert_eq!(events.len(), 1, "one enter, not one record per step");
//! ```

mod engine;
mod event;
mod rule;
mod summary;

pub use engine::{worst_severity, RuleEngine, Sample};
pub use event::{format_time, Action, EventKind, EventRecord, Severity};
pub use rule::{Condition, Qualifiers, Rule, Watch};
pub use summary::RuleSummary;
