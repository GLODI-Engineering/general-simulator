//! Extracts each `D` element's two terminal node names from a netlist. `elspice-mna` parses
//! this same information internally (`spice_core::ast::ElementInstance::nodes`) but doesn't
//! expose per-device topology in its public API (only the resulting unknown/input names) —
//! this crate needs the raw node names to know which two MNA unknowns a diode's voltage is
//! the difference of, so it re-parses via `spice-core` directly rather than duplicating any
//! of `elspice-mna`'s own stamping logic.

use std::collections::BTreeMap;

use spice_core::ast::Statement;
use spice_core::{lexer, parser, Dialect};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiodeNodes {
    pub positive: String,
    pub negative: String,
}

pub fn diode_nodes(source: &str, dialect: Dialect) -> BTreeMap<String, DiodeNodes> {
    let lines = lexer::preprocess(source, dialect);
    let mut result = BTreeMap::new();
    for parsed in parser::parse(&lines, dialect) {
        let Ok(Statement::ElementInstance(element)) = parsed else {
            continue;
        };
        if element.device_letter != 'D' {
            continue;
        }
        if let [positive, negative, ..] = element.nodes.as_slice() {
            result.insert(
                element.name.clone(),
                DiodeNodes {
                    positive: positive.clone(),
                    negative: negative.clone(),
                },
            );
        }
    }
    result
}

pub fn is_ground(node: &str) -> bool {
    node == "0" || node.eq_ignore_ascii_case("gnd")
}

/// Finds `node`'s index among `unknowns` (which store node unknowns as `"V(<name>)"`),
/// matching case-insensitively the same way `elspice-mna` itself normalizes node names.
/// `None` for a ground node, matching `elspice-mna`'s own convention of never allocating an
/// unknown for ground.
pub fn node_index(unknowns: &[String], node: &str) -> Option<usize> {
    if is_ground(node) {
        return None;
    }
    let target = node.to_ascii_uppercase();
    unknowns.iter().position(|unknown| {
        unknown
            .strip_prefix("V(")
            .and_then(|rest| rest.strip_suffix(')'))
            .map(|inner| inner.to_ascii_uppercase())
            == Some(target.clone())
    })
}
