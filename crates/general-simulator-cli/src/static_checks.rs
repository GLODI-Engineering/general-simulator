//! Checks that can be made before the run, from the declaration alone.
//!
//! A rule watches a run; this watches the netlist. The two overlap on purpose — a gate that
//! cannot turn its switch on is reported here if that is provable and by `type=switch_timing`'s
//! never-commutated check if it is not — because a mistake caught before a three-minute
//! transient is cheaper than the same mistake caught after it.
//!
//! The discipline that matters here is restraint. A static check that warns about something it
//! merely cannot prove teaches its reader to ignore it, and a warning nobody reads is worse
//! than no warning, because it occupies the place where a real one would have been seen. So
//! nothing below guesses: a gate driven by a PID, a comparator or a sum of live signals is
//! simply not decidable here, and is left to the runtime check.

use std::collections::BTreeMap;

use general_mna::block_graph::{
    BlockInstance, BlockKind, ConstValue, GainValue, GateBinding, Signal,
};

/// `GateBinding` turns its block's output into a switch state by thresholding it here.
///
/// Not a parameter of the switch: `v_th` on a `kind=ideal_switch` line is the *body diode's*
/// forward threshold and the gate path never consults it. Hard-coded in `general-mna`'s
/// `GateBinding::resolve`, and mirrored here because this check has to agree with it.
const GATE_THRESHOLD: f64 = 0.5;

/// How deep to follow a chain of constant-folding blocks before giving up.
///
/// A real gate chain is two or three blocks. The limit is what stops a cycle among
/// `Signal::Block` edges from recursing forever — the block graph rejects a genuine algebraic
/// loop before any step runs, but this check runs before that validation.
const MAX_DEPTH: usize = 8;

/// Every gate whose driving signal provably never reaches the threshold.
///
/// Returns one message per such gate, phrased for stderr.
pub fn dead_gates(gates: &BTreeMap<String, GateBinding>, blocks: &[BlockInstance]) -> Vec<String> {
    let by_name: BTreeMap<&str, &BlockInstance> =
        blocks.iter().map(|b| (b.name.as_str(), b)).collect();

    let mut found = Vec::new();
    for (switch, binding) in gates {
        for source in binding.source_blocks().into_iter().flatten() {
            let Some(Constant { value, origin }) = constant_output(&by_name, source, 0) else {
                continue;
            };
            if value >= GATE_THRESHOLD {
                continue;
            }
            // Naming the block that *fixed* the value, not only the one the gate points at:
            // a gate is always driven through a `sig2phys` converter, so `ctrl=` names the
            // converter and the mistake is almost always further up the chain.
            let through = if origin == source {
                String::new()
            } else {
                format!(" (through '{source}')")
            };
            found.push(format!(
                "warning: switch '{switch}' can never conduct: its gate is fixed at {value} by \
                 '{origin}'{through}, and a gate turns on at {GATE_THRESHOLD}. The run will \
                 simulate this switch as permanently open -- which still charges and still \
                 settles, so the waveform will not look wrong. If that is deliberate, leave a \
                 comment saying so; if not, check `ctrl=` and the modulator. (`v_th` on the \
                 switch line is its body diode's threshold, not the gate's.)"
            ));
        }
    }
    found
}

/// A provably fixed output, and the block whose declaration fixed it.
struct Constant {
    value: f64,
    /// The deepest block the fold reached -- the constant, the gain, the saturation. This is
    /// what a reader has to edit, which is rarely the block the gate names.
    origin: String,
}

/// What `name`'s output provably is, every step, or `None` when that is not decidable here.
///
/// `None` is the answer for most of the library and that is correct: a PID's output, a
/// comparator's, anything reading the circuit. Only the blocks whose output is fixed by their
/// own declaration are folded.
fn constant_output(
    blocks: &BTreeMap<&str, &BlockInstance>,
    name: &str,
    depth: usize,
) -> Option<Constant> {
    if depth >= MAX_DEPTH {
        return None;
    }
    let block = blocks.get(name)?;
    let input = |index: usize| -> Option<Constant> {
        match block.inputs.get(index)? {
            Signal::Block(from) => constant_output(blocks, from, depth + 1),
            // A previous-step reference starts at rest and then follows a signal this check
            // cannot see, so it is not a constant even when its source is one.
            _ => None,
        }
    };
    let here = |value: f64| {
        Some(Constant {
            value,
            origin: name.to_string(),
        })
    };

    match &block.kind {
        BlockKind::Const(ConstValue::Scalar(value)) => here(*value),
        // A gate is a scalar, so a vector constant driving one is a different error, reported
        // elsewhere; here it is simply not a number this check can compare.
        BlockKind::Const(ConstValue::Vector(_)) => None,
        BlockKind::Gain(GainValue::Scalar(gain)) => here(gain * input(0)?.value),
        BlockKind::Gain(GainValue::Matrix(_)) => None,
        BlockKind::Sum(signs) => {
            let mut total = 0.0;
            for (index, sign) in signs.iter().enumerate() {
                total += sign * input(index)?.value;
            }
            here(total)
        }
        BlockKind::Saturation(limit) => here(input(0)?.value.clamp(-*limit, *limit)),
        // The converter a gate is always driven through: it passes its input along, so the
        // chain continues rather than stopping here.
        BlockKind::Sig2Phys { .. } => input(0),
        // Everything else: not decidable from the declaration. Deliberately silent.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use general_mna::block_graph::PhysicalDomain;

    fn block(name: &str, kind: BlockKind, inputs: &[&str]) -> BlockInstance {
        BlockInstance {
            name: name.to_string(),
            kind,
            inputs: inputs
                .iter()
                .map(|from| Signal::Block((*from).to_string()))
                .collect(),
            ic: None,
        }
    }

    /// A gate chain: `<source> -> GATE (sig2phys) -> switch DSW`.
    fn gated(source: Vec<BlockInstance>) -> (BTreeMap<String, GateBinding>, Vec<BlockInstance>) {
        let last = source.last().expect("a source block").name.clone();
        let mut blocks = source;
        blocks.push(block(
            "GATE",
            BlockKind::Sig2Phys {
                domain: PhysicalDomain::Voltage,
            },
            &[last.as_str()],
        ));
        (
            BTreeMap::from([("DSW".to_string(), GateBinding::Block("GATE".to_string()))]),
            blocks,
        )
    }

    #[test]
    fn a_gate_pinned_below_the_threshold_is_reported() {
        let (gates, blocks) = gated(vec![block(
            "DEAD",
            BlockKind::Const(ConstValue::Scalar(0.0)),
            &[],
        )]);
        let found = dead_gates(&gates, &blocks);
        assert_eq!(found.len(), 1, "{found:?}");
        // The block that fixed the value, not the one `ctrl=` names: a gate is always driven
        // through a converter, so the mistake is almost always further up.
        assert!(found[0].contains("fixed at 0 by 'DEAD'"), "{}", found[0]);
        assert!(found[0].contains("through 'GATE'"), "{}", found[0]);
    }

    #[test]
    fn a_gate_that_can_reach_the_threshold_is_not_reported() {
        let (gates, blocks) = gated(vec![block(
            "LIVE",
            BlockKind::Const(ConstValue::Scalar(0.6)),
            &[],
        )]);
        assert!(dead_gates(&gates, &blocks).is_empty());
    }

    #[test]
    fn a_constant_scaled_down_past_the_threshold_is_still_decidable() {
        let (gates, blocks) = gated(vec![
            block("ONE", BlockKind::Const(ConstValue::Scalar(1.0)), &[]),
            block("SMALL", BlockKind::Gain(GainValue::Scalar(0.2)), &["ONE"]),
        ]);
        let found = dead_gates(&gates, &blocks);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("fixed at 0.2"), "{}", found[0]);
    }

    #[test]
    fn a_constant_scaled_up_past_the_threshold_is_silent() {
        let (gates, blocks) = gated(vec![
            block("ONE", BlockKind::Const(ConstValue::Scalar(1.0)), &[]),
            block("BIG", BlockKind::Gain(GainValue::Scalar(0.8)), &["ONE"]),
        ]);
        assert!(dead_gates(&gates, &blocks).is_empty());
    }

    #[test]
    fn a_sum_of_constants_is_decidable() {
        let (gates, blocks) = gated(vec![
            block("A", BlockKind::Const(ConstValue::Scalar(1.0)), &[]),
            block("B", BlockKind::Const(ConstValue::Scalar(0.9)), &[]),
            block("DIFF", BlockKind::Sum(vec![1.0, -1.0]), &["A", "B"]),
        ]);
        let found = dead_gates(&gates, &blocks);
        assert_eq!(found.len(), 1, "1 - 0.9 is below the threshold: {found:?}");
    }

    #[test]
    fn a_signal_this_check_cannot_decide_produces_no_warning() {
        // The restraint that makes the check worth having. A PWM's output is not foldable here,
        // and a warning about something merely unproven teaches its reader to ignore the rest.
        let (gates, blocks) = gated(vec![
            block("D", BlockKind::Const(ConstValue::Scalar(0.4)), &[]),
            block(
                "PWM1",
                BlockKind::Pwm {
                    freq_hz: 100e3,
                    red: 0.0,
                    fed: 0.0,
                    output_names: Vec::new(),
                },
                &["D"],
            ),
        ]);
        assert!(
            dead_gates(&gates, &blocks).is_empty(),
            "a 0.4 duty is not a 0.4 output"
        );
    }

    #[test]
    fn a_gate_naming_a_block_that_does_not_exist_is_left_to_the_real_validator() {
        let gates = BTreeMap::from([("DSW".to_string(), GateBinding::Block("NOPE".to_string()))]);
        assert!(dead_gates(&gates, &[]).is_empty());
    }
}
