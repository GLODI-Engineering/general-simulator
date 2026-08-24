//! Extracts each `D` element's two terminal node names from an already-parsed (and, per
//! `general-mna`'s `hierarchy::flatten`, already hierarchy-resolved) statement list.
//! `general-mna` parses this same information internally
//! (`general_spice_core::ast::ElementInstance::nodes`) but doesn't expose per-device topology in
//! its public API (only the resulting unknown/input names) — this crate needs the raw node
//! names to know which two MNA unknowns a diode's voltage is the difference of. Operating on the
//! caller's own already-parsed `statements` (rather than re-parsing raw text here) is what keeps
//! this in sync with whatever `MnaSystem` the caller built from that same statement list — in
//! particular, a diode declared inside a `.subckt` body only exists under its flattened,
//! dotted-path name (e.g. `X1.D1`), which a fresh from-scratch text re-parse would never see.

use std::collections::BTreeMap;

use general_spice_core::ast::Statement;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiodeNodes {
    pub positive: String,
    pub negative: String,
}

pub fn diode_nodes(statements: &[Statement]) -> BTreeMap<String, DiodeNodes> {
    let mut result = BTreeMap::new();
    for statement in statements {
        let Statement::ElementInstance(element) = statement else {
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
/// matching case-insensitively the same way `general-mna` itself normalizes node names.
/// `None` for a ground node, matching `general-mna`'s own convention of never allocating an
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
