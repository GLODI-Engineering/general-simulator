//! What each column of a run *is*, beside its name — the solver's half of the signal-metadata
//! convention written up in `book/user-guide/src/signal-metadata.md`.
//!
//! Two rules decide everything here, and they are the two the book states:
//!
//! 1. **A quantity's nature travels beside its name, never inside it.** A column's name is an
//!    interface — `kind=measure` names columns, scripts name columns, a spreadsheet opens the
//!    CSV header — so no name changes to carry a unit. The `.gsim_samples` footer's
//!    [`gsim_samples::VectorMeta`] already has the fields; this module is what finally puts
//!    something in them.
//! 2. **Nothing is inferred from how a name is spelled.** What a column is comes from what it
//!    is *in the run*: the sweep axis, an unknown of the MNA system, or a block's output. The
//!    `V(`/`I(` split within the MNA unknowns is not an exception — `general-mna` generates
//!    those names itself, one per node then one per branch, so reading that prefix is reading a
//!    name this toolchain wrote to a fixed shape, not guessing at a name a user chose. A column
//!    is never classified by its prefix before its position has already said it is an MNA
//!    unknown.
//!
//! The third rule is about what *not* to say. An empty unit means "not known", never
//! "dimensionless", and a block output is where that matters: a `kind=pid`'s output could be a
//! voltage command, a duty cycle or a modulation index, and the netlist does not say which. Such
//! a column reports `quantity=signal` with no unit, which is a positive statement — *this is a
//! signal-flow value, not a potential* — and is exactly what stops a consumer printing a duty
//! cycle of 0.42 as 0.42 volts. Two block kinds are the exception, because for them the netlist
//! *does* fix the unit: `kind=phys2sig` names the node or branch it reads, and `kind=sig2phys`
//! declares the domain it drives.

use dae_runtime::{BlockInstance, BlockKind, Phys2SigTarget, PhysicalDomain};

/// What kind of quantity a column holds. The whole vocabulary — deliberately four entries, since
/// a vocabulary a consumer cannot enumerate is as good as no vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Quantity {
    /// The sweep axis.
    Time,
    /// An electrical potential: a node voltage, or a block output the netlist has tied to one.
    Voltage,
    /// An electrical current through a branch, or a block output the netlist has tied to one.
    Current,
    /// A signal-flow value. Its unit is not knowable from the netlist, and saying so is the
    /// point of this variant — see this module's own doc comment.
    Signal,
}

impl Quantity {
    /// The `quantity` field's value in a `.gsim_samples` footer.
    pub fn as_str(self) -> &'static str {
        match self {
            Quantity::Time => "time",
            Quantity::Voltage => "voltage",
            Quantity::Current => "current",
            Quantity::Signal => "signal",
        }
    }

    /// The SI unit symbol, or `None` when the netlist does not fix one. `None` is written out as
    /// an empty `unit` field, which the convention defines as "not known".
    pub fn unit(self) -> Option<&'static str> {
        match self {
            Quantity::Time => Some("s"),
            Quantity::Voltage => Some("V"),
            Quantity::Current => Some("A"),
            Quantity::Signal => None,
        }
    }

    /// The SPICE rawfile `Variables:` type string.
    ///
    /// The rawfile has no generic-signal type, and the readers this crate was validated against
    /// reject a type outside their own four-entry table, so a signal-flow column is written as
    /// `voltage` there — see `raw_format`'s own "Variable-type compatibility caveat". That is a
    /// loss the format forces, and it is why it is spelled out here as a conversion away from a
    /// known classification rather than left as a guess at the column's name: the `.gsim_samples`
    /// footer still says `signal`, and this is the one output that cannot.
    pub fn raw_variable_type(self) -> &'static str {
        match self {
            Quantity::Time => "time",
            Quantity::Current => "current",
            Quantity::Voltage | Quantity::Signal => "voltage",
        }
    }
}

/// Which analysis produced a run, for the footer's `analysis` field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Analysis {
    Transient,
    Dc,
}

impl Analysis {
    pub fn as_str(self) -> &'static str {
        match self {
            Analysis::Transient => "transient",
            Analysis::Dc => "dc",
        }
    }
}

/// Classifies every column of `headers`.
///
/// `headers[0]` is the sweep axis and `headers[1..=circuit_unknowns]` are the MNA system's own
/// unknowns, by the same convention `Waveform` and every writer in this crate already follow.
/// Anything after that is a block output column: a block's own name, one of the extra output
/// channels a `cscript`/`pmsm`/`pwm`/... block registers, or `NAME[i]` for one element of a
/// vector-valued signal.
pub fn quantities(
    headers: &[String],
    circuit_unknowns: usize,
    blocks: &[BlockInstance],
) -> Vec<Quantity> {
    headers
        .iter()
        .enumerate()
        .map(|(column, name)| {
            if column == 0 {
                Quantity::Time
            } else if column <= circuit_unknowns {
                mna_unknown_quantity(name)
            } else {
                block_column_quantity(name, blocks)
            }
        })
        .collect()
}

/// An MNA unknown is a node potential or a branch current, and `general-mna` spells which one it
/// generated into the name — see this module's doc comment on why reading that is not inference.
/// A name in neither shape would mean that convention changed under us, and claiming a unit for
/// it would be the guess this module exists to avoid.
fn mna_unknown_quantity(name: &str) -> Quantity {
    if name.starts_with("V(") {
        Quantity::Voltage
    } else if name.starts_with("I(") {
        Quantity::Current
    } else {
        Quantity::Signal
    }
}

/// A block output column. `signal` unless the netlist itself fixed the unit, which only
/// `kind=phys2sig` (it names the node or branch it reads) and `kind=sig2phys` (it declares the
/// domain it drives) do.
fn block_column_quantity(name: &str, blocks: &[BlockInstance]) -> Quantity {
    let block_name = name.split_once('[').map_or(name, |(head, _)| head);
    let Some(block) = blocks.iter().find(|b| b.name == block_name) else {
        // An extra output channel of a multi-output block (`PMSM:id` and friends). Those are
        // signal-flow values, and none of them is a converter.
        return Quantity::Signal;
    };
    match &block.kind {
        BlockKind::Phys2Sig(Phys2SigTarget::Voltage(_)) => Quantity::Voltage,
        BlockKind::Phys2Sig(Phys2SigTarget::Current(_)) => Quantity::Current,
        BlockKind::Sig2Phys {
            domain: PhysicalDomain::Voltage,
        } => Quantity::Voltage,
        BlockKind::Sig2Phys {
            domain: PhysicalDomain::Current,
        } => Quantity::Current,
        _ => Quantity::Signal,
    }
}

/// The `.gsim_samples` footer's `(sweep, signals)` metadata for a run's columns.
pub fn vector_meta(
    headers: &[String],
    circuit_unknowns: usize,
    blocks: &[BlockInstance],
    analysis: Analysis,
) -> (gsim_samples::VectorMeta, Vec<gsim_samples::VectorMeta>) {
    let kinds = quantities(headers, circuit_unknowns, blocks);
    let meta = |name: &str, quantity: Quantity| {
        let mut m = gsim_samples::VectorMeta::new(name)
            .with_quantity(quantity.as_str())
            .with_analysis(analysis.as_str());
        if let Some(unit) = quantity.unit() {
            m = m.with_unit(unit);
        }
        m
    };
    let sweep = match (headers.first(), kinds.first()) {
        (Some(name), Some(&quantity)) => meta(name, quantity),
        // No columns at all: `Writer::new` rejects that anyway, with its own error.
        _ => meta("t", Quantity::Time),
    };
    let signals = headers
        .iter()
        .zip(&kinds)
        .skip(1)
        .map(|(name, &quantity)| meta(name, quantity))
        .collect();
    (sweep, signals)
}

#[cfg(test)]
mod tests {
    use super::*;
    use dae_runtime::ConstValue;

    fn block(name: &str, kind: BlockKind) -> BlockInstance {
        BlockInstance {
            name: name.to_string(),
            kind,
            inputs: Vec::new(),
            ic: None,
        }
    }

    /// The classification is positional first and only then looks at a name, so a block whose
    /// own name happens to start with `V(`-like text cannot be mistaken for a node voltage, and
    /// a node is never read as a block.
    #[test]
    fn position_decides_before_any_name_is_read() {
        let headers: Vec<String> = ["t", "V(out)", "I(V1)", "DUTY"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let blocks = [block("DUTY", BlockKind::Const(ConstValue::Scalar(0.42)))];
        assert_eq!(
            quantities(&headers, 2, &blocks),
            vec![
                Quantity::Time,
                Quantity::Voltage,
                Quantity::Current,
                Quantity::Signal
            ]
        );
        // The same names with nothing classified as a circuit unknown: every one of them is a
        // block column, and `V(out)`'s spelling buys it nothing.
        assert_eq!(
            quantities(&headers, 0, &blocks),
            vec![
                Quantity::Time,
                Quantity::Signal,
                Quantity::Signal,
                Quantity::Signal
            ]
        );
    }

    /// A `phys2sig` reading a node is volts and one reading a branch is amps, because the
    /// netlist said which — the one case where a block output's unit is knowable.
    #[test]
    fn a_converter_block_reports_the_unit_the_netlist_fixed() {
        let blocks = [
            block(
                "VPROBE",
                BlockKind::Phys2Sig(Phys2SigTarget::Voltage("out".to_string())),
            ),
            block(
                "IPROBE",
                BlockKind::Phys2Sig(Phys2SigTarget::Current("L1".to_string())),
            ),
            block(
                "DRIVE",
                BlockKind::Sig2Phys {
                    domain: PhysicalDomain::Current,
                },
            ),
        ];
        let headers: Vec<String> = ["t", "VPROBE", "IPROBE", "DRIVE"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            quantities(&headers, 0, &blocks),
            vec![
                Quantity::Time,
                Quantity::Voltage,
                Quantity::Current,
                Quantity::Current
            ]
        );
    }

    /// One element of a vector-valued block signal is the same quantity as the block.
    #[test]
    fn a_vector_element_inherits_its_block() {
        let blocks = [block(
            "ABC",
            BlockKind::Const(ConstValue::Vector(vec![1.0, 2.0])),
        )];
        let headers: Vec<String> = ["t", "ABC[0]", "ABC[1]"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(
            quantities(&headers, 0, &blocks),
            vec![Quantity::Time, Quantity::Signal, Quantity::Signal]
        );
    }

    /// An unknown unit is written as an absent one, and `signal` is what carries the meaning.
    #[test]
    fn an_unknown_unit_is_empty_and_the_quantity_still_says_what_it_is() {
        let blocks = [block("DUTY", BlockKind::Const(ConstValue::Scalar(0.42)))];
        let headers: Vec<String> = ["t", "V(out)", "DUTY"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (sweep, signals) = vector_meta(&headers, 1, &blocks, Analysis::Transient);
        assert_eq!(
            (sweep.quantity.as_str(), sweep.unit.as_str()),
            ("time", "s")
        );
        assert_eq!(sweep.analysis, "transient");
        assert_eq!(
            (signals[0].quantity.as_str(), signals[0].unit.as_str()),
            ("voltage", "V")
        );
        assert_eq!(
            (signals[1].quantity.as_str(), signals[1].unit.as_str()),
            ("signal", "")
        );
        // The names are untouched, which is the compatibility half of the convention.
        assert_eq!(signals[0].name, "V(out)");
        assert_eq!(signals[1].name, "DUTY");
    }

    /// Only the rawfile is allowed to flatten `signal` onto `voltage`, and nothing else does.
    #[test]
    fn the_rawfile_type_is_the_only_place_signal_is_flattened() {
        assert_eq!(Quantity::Signal.raw_variable_type(), "voltage");
        assert_eq!(Quantity::Signal.as_str(), "signal");
        assert_eq!(Quantity::Signal.unit(), None);
        assert_eq!(Quantity::Current.raw_variable_type(), "current");
        assert_eq!(Quantity::Time.raw_variable_type(), "time");
    }
}
