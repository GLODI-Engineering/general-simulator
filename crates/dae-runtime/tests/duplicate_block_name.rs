//! `DaeError::DuplicateBlockName` — two blocks resolving to the same name (either two
//! `BlockInstance`s sharing a `.name`, or a `Pwm`/`PhaseShiftPwm`/`CScript`/`CoordinateTransform`/
//! `Pmsm`'s own secondary `output_names` alias colliding with another block's name) used to be
//! silently resolved by `BTreeMap::insert` letting the later one win — a `Signal::Block(name)`
//! reference could silently resolve to the wrong block with no error at all. Verified here
//! rejected up front, before any step is solved, same as `DaeError::AlgebraicLoop`.

use std::collections::BTreeMap;

use dae_runtime::{
    simulate_transient_with_blocks, BlockInstance, BlockKind, ConstValue, DaeError, Signal,
    TimeStep,
};
use general_spice_core::Dialect;
use pwl_devices::{IdealDiode, IdealSwitch};

const NETLIST: &str = "V1 a 0 5\nD1 a b idealswitchmodel\nR1 b 0 1000";

fn dummy_ideal_switches() -> BTreeMap<String, IdealSwitch> {
    let mut m = BTreeMap::new();
    m.insert(
        "D1".to_string(),
        IdealSwitch::new(0.1, IdealDiode::new(0.0, -100.0, 0.0, 1e6, 0.0)),
    );
    m
}

fn run(blocks: &[BlockInstance]) -> Result<Vec<dae_runtime::TransientWithBlocksStep>, DaeError> {
    let ideal_switches = dummy_ideal_switches();
    let diodes = BTreeMap::new();
    let gates = BTreeMap::new();
    simulate_transient_with_blocks(
        NETLIST,
        Dialect::Ngspice,
        &diodes,
        &ideal_switches,
        blocks,
        &gates,
        0.1,
        None,
        1e-5,
        TimeStep::Fixed(1e-6),
    )
}

#[test]
fn two_blocks_sharing_a_name_are_rejected() {
    let blocks = vec![
        BlockInstance {
            name: "A".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(1.0)),
            inputs: vec![],
        },
        BlockInstance {
            name: "A".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(2.0)),
            inputs: vec![],
        },
    ];

    let err = run(&blocks).unwrap_err();
    assert_eq!(err, DaeError::DuplicateBlockName("A".to_string()));
}

#[test]
fn a_pwm_complement_alias_colliding_with_another_blocks_name_is_rejected() {
    // MOD's own complement output is aliased to "TAKEN" via outputs=, which is also another
    // block's own declared name -- exactly the silent-misroute scenario this check exists for.
    let blocks = vec![
        BlockInstance {
            name: "DUTY".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(0.5)),
            inputs: vec![],
        },
        BlockInstance {
            name: "TAKEN".to_string(),
            kind: BlockKind::Const(ConstValue::Scalar(3.0)),
            inputs: vec![],
        },
        BlockInstance {
            name: "MOD".to_string(),
            kind: BlockKind::Pwm {
                freq_hz: 100_000.0,
                red: 0.0,
                fed: 0.0,
                output_names: vec!["MOD".to_string(), "TAKEN".to_string()],
            },
            inputs: vec![Signal::Block("DUTY".to_string())],
        },
    ];

    let err = run(&blocks).unwrap_err();
    assert_eq!(err, DaeError::DuplicateBlockName("TAKEN".to_string()));
}
