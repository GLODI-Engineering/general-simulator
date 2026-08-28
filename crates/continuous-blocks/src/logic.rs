//! Digital/logic signal blocks: combinational gates ([`LogicOp`]), an SR/fault latch
//! (level-triggered, [`LatchPriority`]), and the shared edge-detection + next-state rules
//! edge-triggered flip-flops/counters both need ([`FlipFlopKind`]). See
//! `book/dev-guide/src/logic-signals.md` for the full design survey this module implements.
//!
//! Every input/output here is an ordinary `f64` `Scalar` signal, thresholded at `>= 0.5` the
//! same way `GateBinding`/[`crate::Hysteresis`] already read a "boolean" signal — there is no
//! dedicated boolean `SignalValue` variant anywhere in this workspace, and this module doesn't
//! introduce one; see the design doc's own "The type" section for why.

/// Reads an ordinary `f64` signal as a logic level: `true` iff `x >= 0.5`, the same threshold
/// `GateBinding::resolve` already uses.
pub fn as_bool(x: f64) -> bool {
    x >= 0.5
}

/// The inverse of [`as_bool`] — the canonical `f64` encoding of a logic level.
pub fn from_bool(b: bool) -> f64 {
    if b {
        1.0
    } else {
        0.0
    }
}

/// A rising edge occurred between the previous and current `clk` sample: was below threshold,
/// is now at or above it. Shared by every edge-triggered block in this module
/// ([`FlipFlopKind`]-driven flip-flops, and `dae-runtime`'s own `Counter` block, which reuses
/// this same function rather than duplicating the comparison).
pub fn rising_edge(clk: f64, prev_clk: f64) -> bool {
    as_bool(clk) && !as_bool(prev_clk)
}

/// One of the combinational logic gates — `kind=and`/`or`/`xor`/`nand`/`nor`/`xnor` (N-input,
/// `N >= 2`) or `kind=not` (exactly 1 input). See [`LogicOp::call`] for the reduction rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogicOp {
    And,
    Or,
    Xor,
    Nand,
    Nor,
    Xnor,
    Not,
}

impl LogicOp {
    pub fn from_name(name: &str) -> Option<Self> {
        use LogicOp::*;
        Some(match name {
            "and" => And,
            "or" => Or,
            "xor" => Xor,
            "nand" => Nand,
            "nor" => Nor,
            "xnor" => Xnor,
            "not" => Not,
            _ => return None,
        })
    }

    /// The exact inverse of [`Self::from_name`] — see `MathFn1::name`'s own doc comment for why
    /// this exists (naming *this specific* op for diagnostics, not the family as a whole).
    pub fn name(self) -> &'static str {
        use LogicOp::*;
        match self {
            And => "and",
            Or => "or",
            Xor => "xor",
            Nand => "nand",
            Nor => "nor",
            Xnor => "xnor",
            Not => "not",
        }
    }

    /// `true` iff this op requires exactly one input (`Not`) rather than `N >= 2`.
    pub fn is_unary(self) -> bool {
        matches!(self, LogicOp::Not)
    }

    /// Reduces `inputs` (already thresholded to `bool` by the caller, via [`as_bool`]) per this
    /// op's own rule:
    /// - `And`/`Nand`: `true` (negated for `Nand`) iff every input is `true`.
    /// - `Or`/`Nor`: `true` (negated for `Nor`) iff at least one input is `true`.
    /// - `Xor`/`Xnor`: `true` (negated for `Xnor`) iff an *odd* number of inputs are `true` —
    ///   the standard N-input generalization, not "exactly one" (which doesn't generalize past
    ///   two inputs in a way anyone actually expects).
    /// - `Not`: `inputs` must have exactly one element; `true` iff it's `false`.
    ///
    /// # Panics
    /// If `Not` is called with a slice that isn't exactly length 1, or a non-`Not` op is called
    /// with an empty slice — both unreachable in practice, since the caller (`dae-runtime`'s own
    /// `evaluate_blocks`) already validates arity against [`Self::is_unary`] before calling this.
    pub fn call(self, inputs: &[bool]) -> bool {
        use LogicOp::*;
        match self {
            And => inputs.iter().all(|&b| b),
            Nand => !inputs.iter().all(|&b| b),
            Or => inputs.iter().any(|&b| b),
            Nor => !inputs.iter().any(|&b| b),
            Xor => inputs.iter().filter(|&&b| b).count() % 2 == 1,
            Xnor => inputs.iter().filter(|&&b| b).count() % 2 == 0,
            Not => {
                assert_eq!(inputs.len(), 1, "LogicOp::Not requires exactly one input");
                !inputs[0]
            }
        }
    }
}

/// How an [`srlatch_next`] resolves the `set && reset` (both asserted at once) case — see
/// `logic-signals.md`'s own Category 2 for the full reasoning behind `Set` as the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LatchPriority {
    /// `set && reset` -> latched `true`. The default: a fault-latch use case should still latch
    /// a fault that occurs in the same step a reset is asserted, not silently clear it.
    Set,
    /// `set && reset` -> latched `false`. For modeling a real reset-dominant latch chip.
    Reset,
}

impl LatchPriority {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "set" => Some(LatchPriority::Set),
            "reset" => Some(LatchPriority::Reset),
            _ => None,
        }
    }
}

/// One step of an SR (set/reset) latch — level-triggered, no clock: `set`/`reset` act every
/// step, immediately. `q` is the latch's own current state (before this step); returns the new
/// state.
///
/// ```text
/// set,   reset  -> q_next
/// false, false  -> q (hold)
/// true,  false  -> true
/// false, true   -> false
/// true,  true   -> per `priority`
/// ```
pub fn srlatch_next(q: bool, set: bool, reset: bool, priority: LatchPriority) -> bool {
    match (set, reset) {
        (false, false) => q,
        (true, false) => true,
        (false, true) => false,
        (true, true) => matches!(priority, LatchPriority::Set),
    }
}

/// One of the edge-triggered flip-flop next-state rules — `kind=dff`/`jkff`/`tff`. All three
/// share one `BlockState` shape in `dae-runtime` (`{ q: bool, prev_clk: f64 }`) and are only
/// ever evaluated (via [`Self::next_state`]) at the instant [`rising_edge`] reports `true`;
/// `dae-runtime` holds `q` unchanged on every other step, exactly like every other zero-order-
/// hold block already does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlipFlopKind {
    D,
    Jk,
    T,
}

impl FlipFlopKind {
    pub fn from_name(name: &str) -> Option<Self> {
        use FlipFlopKind::*;
        Some(match name {
            "dff" => D,
            "jkff" => Jk,
            "tff" => T,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        use FlipFlopKind::*;
        match self {
            D => "dff",
            Jk => "jkff",
            T => "tff",
        }
    }

    /// How many data inputs (besides `clk`) this variant needs: `D`/`T` take one (`d`/`t`); `Jk`
    /// takes two (`j`, `k`).
    pub fn input_count(self) -> usize {
        match self {
            FlipFlopKind::D | FlipFlopKind::T => 1,
            FlipFlopKind::Jk => 2,
        }
    }

    /// The next-state rule, evaluated only at a detected rising edge (see [`rising_edge`]).
    /// `inputs` has exactly [`Self::input_count`] elements, already thresholded to `bool`.
    ///
    /// - `D`: `q_next = inputs[0]`.
    /// - `Jk`: the classic table — `(j,k) = (false,false)` hold, `(true,false)` set,
    ///   `(false,true)` reset, `(true,true)` toggle.
    /// - `T`: `q_next = q XOR inputs[0]` — toggles when asserted, holds otherwise.
    ///
    /// # Panics
    /// If `inputs.len() != self.input_count()` — unreachable in practice, the caller's own
    /// responsibility to uphold (checked once, at parse time, by `general-mna`'s own netlist
    /// field count, not re-checked here every step).
    pub fn next_state(self, q: bool, inputs: &[bool]) -> bool {
        match self {
            FlipFlopKind::D => {
                assert_eq!(inputs.len(), 1);
                inputs[0]
            }
            FlipFlopKind::T => {
                assert_eq!(inputs.len(), 1);
                q ^ inputs[0]
            }
            FlipFlopKind::Jk => {
                assert_eq!(inputs.len(), 2);
                let (j, k) = (inputs[0], inputs[1]);
                match (j, k) {
                    (false, false) => q,
                    (true, false) => true,
                    (false, true) => false,
                    (true, true) => !q,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn as_bool_and_from_bool_round_trip_at_the_gate_binding_threshold() {
        assert!(!as_bool(0.4));
        assert!(as_bool(0.5));
        assert!(as_bool(0.6));
        assert_eq!(from_bool(true), 1.0);
        assert_eq!(from_bool(false), 0.0);
    }

    #[test]
    fn rising_edge_fires_only_on_the_low_to_high_transition() {
        assert!(rising_edge(1.0, 0.0)); // low -> high: the one true case
        assert!(!rising_edge(1.0, 1.0)); // already high: not an edge
        assert!(!rising_edge(0.0, 0.0)); // stays low
        assert!(!rising_edge(0.0, 1.0)); // falling, not rising
    }

    #[test]
    fn and_or_nand_nor_match_the_truth_table() {
        assert!(LogicOp::And.call(&[true, true, true]));
        assert!(!LogicOp::And.call(&[true, false, true]));
        assert!(!LogicOp::Nand.call(&[true, true, true]));
        assert!(LogicOp::Nand.call(&[true, false, true]));

        assert!(!LogicOp::Or.call(&[false, false, false]));
        assert!(LogicOp::Or.call(&[false, true, false]));
        assert!(LogicOp::Nor.call(&[false, false, false]));
        assert!(!LogicOp::Nor.call(&[false, true, false]));
    }

    #[test]
    fn xor_xnor_generalize_to_odd_parity_past_two_inputs() {
        // 2-input, the textbook case.
        assert!(LogicOp::Xor.call(&[true, false]));
        assert!(!LogicOp::Xor.call(&[true, true]));
        // 3-input: two true is even parity (false), three true is odd parity (true) -- neither
        // is "exactly one," confirming this really is odd-parity, not a naive extension.
        assert!(!LogicOp::Xor.call(&[true, true, false]));
        assert!(LogicOp::Xor.call(&[true, true, true]));
        assert!(LogicOp::Xnor.call(&[true, true, false]));
        assert!(!LogicOp::Xnor.call(&[true, true, true]));
    }

    #[test]
    fn not_inverts_its_one_input() {
        assert!(!LogicOp::Not.call(&[true]));
        assert!(LogicOp::Not.call(&[false]));
        assert!(LogicOp::Not.is_unary());
        assert!(!LogicOp::And.is_unary());
    }

    #[test]
    #[should_panic]
    fn not_panics_on_the_wrong_arity() {
        LogicOp::Not.call(&[true, false]);
    }

    #[test]
    fn srlatch_holds_sets_resets_and_resolves_the_priority_case() {
        assert!(!srlatch_next(false, false, false, LatchPriority::Set)); // hold false
        assert!(srlatch_next(true, false, false, LatchPriority::Set)); // hold true
        assert!(srlatch_next(false, true, false, LatchPriority::Set)); // set
        assert!(!srlatch_next(true, false, true, LatchPriority::Set)); // reset
                                                                       // both asserted: priority decides, independent of q's own prior value.
        assert!(srlatch_next(false, true, true, LatchPriority::Set));
        assert!(!srlatch_next(true, true, true, LatchPriority::Reset));
    }

    #[test]
    fn dff_next_state_is_just_d() {
        assert!(FlipFlopKind::D.next_state(false, &[true]));
        assert!(!FlipFlopKind::D.next_state(true, &[false]));
    }

    #[test]
    fn jkff_matches_the_classic_four_row_table() {
        let jk = FlipFlopKind::Jk;
        assert!(!jk.next_state(false, &[false, false])); // hold (was false)
        assert!(jk.next_state(true, &[false, false])); // hold (was true)
        assert!(jk.next_state(false, &[true, false])); // set
        assert!(!jk.next_state(true, &[false, true])); // reset
        assert!(jk.next_state(false, &[true, true])); // toggle false->true
        assert!(!jk.next_state(true, &[true, true])); // toggle true->false
    }

    #[test]
    fn tff_toggles_on_t_and_holds_otherwise() {
        let t = FlipFlopKind::T;
        assert!(!t.next_state(false, &[false])); // hold
        assert!(t.next_state(false, &[true])); // toggle
        assert!(!t.next_state(true, &[true])); // toggle back
    }

    #[test]
    fn from_name_matches_conventional_names_and_rejects_unknown() {
        assert_eq!(LogicOp::from_name("xor"), Some(LogicOp::Xor));
        assert_eq!(LogicOp::from_name("nope"), None);
        assert_eq!(FlipFlopKind::from_name("jkff"), Some(FlipFlopKind::Jk));
        assert_eq!(
            LatchPriority::from_name("reset"),
            Some(LatchPriority::Reset)
        );
    }
}
