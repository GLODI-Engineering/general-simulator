//! A switching node may never leave the window its own body diodes clamp it to, at any step
//! size. Regression fixture for the clamp overshoot: a node held by a conducting body diode
//! drifted tens of volts past the clamp, and which step size showed it depended on the
//! circuit, so a voltage-stress figure read off such a run was wrong by up to ~30 V here (and
//! ~150 V in the report that prompted this).
//!
//! ## The circuit
//!
//! A generic three-leg bridge on a 400 V rail. Every leg is the same textbook half-bridge
//! (`idealswitch` high side, `idealswitch` low side, a capacitor across each switch, an
//! inductor from the switching node to a 200 V mid-rail so the leg always has a load current
//! to commutate), the three legs are gated 120 degrees apart, and each leg's gates actually
//! switch — the one feature the two small circuits in the report that *did* clamp correctly
//! both lacked.
//!
//! The legs differ only in the capacitance across each switch: 174 pF, 1.65 nF, 16.5 nF. That
//! spread is the point. The mode that misbehaves is the stiff one formed by a conducting
//! switch's resistance and the capacitance on its node, with time constant
//!
//! $$\tau = R_{on} \, C_{node} = 0.021\ \Omega \cdot C_{node},$$
//!
//! i.e. $7.3\ \mathrm{ps}$, $69\ \mathrm{ps}$ and $0.69\ \mathrm{ns}$ for the three legs. A
//! given step size only mishandles a leg whose $\tau$ is in the wrong ratio to it, so one
//! circuit with three decades of $\tau$ fails at *every* step size this test runs, just not in
//! the same leg: before the fix the peaks were leg c 427.9 V at 5 ns, leg b 428.1 V at 0.5 ns,
//! leg a 428.1 V at 0.05 ns (and troughs of -25.9 V and -13.8 V against a -1.0 V clamp).
//!
//! ## The window a node is allowed to be in, derived from the device model
//!
//! A gated-off `idealswitch` is its body diode and nothing else. The body diode's forward
//! segment is $v = v_f + i / g_{on}$, so while the high-side body diode conducts the current
//! $i$ its node sits at
//!
//! $$V_{node} = V_{rail} + v_f + \frac{i}{g_{on}},$$
//!
//! and symmetrically the low-side body diode holds the node at $-v_f - i/g_{on}$. With the
//! channel gated on instead, the node sits within $i R_{on}$ of its rail, which is $4.8\times$
//! *tighter* ($R_{on} = 0.021\ \Omega$ against $1/g_{on} = 0.01\ \Omega$ plus a 0.7 V
//! threshold), so the body-diode window bounds both states. The only current a leg's diode has
//! to carry is that leg's inductor current, which the test reads out of the run itself rather
//! than assuming: it is $32.9$ A here, so the window is
//!
//! $$-0.7 - 0.329 \le V_{node} \le 400 + 0.7 + 0.329,$$
//!
//! that is $[-1.03, 401.03]$ V. [`CLAMP_TOLERANCE`] adds half a volt to that for the
//! displacement current the diode also briefly absorbs from the capacitors on its node at the
//! instant it picks the load current up — a real transient, not slack for integration error:
//! the excursions this test exists to catch are 27 V, not half a volt.
//!
//! Note deliberately *not* asserted: the peak the rail's own current reaches. An ideal voltage
//! source feeding a capacitor through a closing switch draws a current that grows without
//! bound as the step shrinks (81 kA at 0.05 ns here), which is a property of the model, not a
//! defect, and says nothing about whether a node is clamped.

use dae_runtime::{simulate_transient_with_ideal_switches, GateState, OperatingPoint};
use general_spice_core::Dialect;
use pwl_devices::{IdealDiode, IdealSwitch};
use std::collections::BTreeMap;

const V_RAIL: f64 = 400.0;
const R_ON: f64 = 0.021;
const V_F: f64 = 0.7;
const G_ON: f64 = 100.0;
const PERIOD: f64 = 1e-6;
const T_FINAL: f64 = 1.6e-6;

/// Allowance over the body diode's own forward drop at the leg's load current, for the
/// displacement current the diode additionally absorbs from the capacitors across the two
/// switches during a commutation. 0.5 V is 50 A through `G_ON` — generous for that, and two
/// orders of magnitude below the excursions this test catches.
const CLAMP_TOLERANCE: f64 = 0.5;

/// How closely the same node's extreme voltage must agree between step sizes. The clamp is a
/// property of the device, so it is the same number at every step size; before the fix the
/// same node's peak moved by 27 V across these three.
const STEP_INDEPENDENCE_TOLERANCE: f64 = 0.5;

const NETLIST: &str = "V1 p 0 400\n\
D1 p a idealswitch\nD2 a 0 idealswitch\nC1 p a 174p\nC2 a 0 174p\n\
La a ma 2.8u\nV2 ma 0 200\n\
D3 p b idealswitch\nD4 b 0 idealswitch\nC3 p b 1.65n\nC4 b 0 1.65n\n\
Lb b mb 2.8u\nV3 mb 0 200\n\
D5 p c idealswitch\nD6 c 0 idealswitch\nC5 p c 16.5n\nC6 c 0 16.5n\n\
Lc c mc 2.8u\nV4 mc 0 200\n";

/// `(switching node, that leg's inductor current)`, one per leg, low capacitance first.
const LEGS: [(&str, &str); 3] = [("a", "I(La)"), ("b", "I(Lb)"), ("c", "I(Lc)")];

/// Each leg's gates: high side on for the first 46% of the period, low side for the last 46%,
/// the two legs after the first shifted a third of a period so the three legs commutate at
/// different times.
fn gate_state(name: &str, t: f64) -> GateState {
    let phase = match name {
        "D3" | "D4" => PERIOD / 3.0,
        "D5" | "D6" => 2.0 * PERIOD / 3.0,
        _ => 0.0,
    };
    let tau = (t - phase).rem_euclid(PERIOD) / PERIOD;
    let high_side = matches!(name, "D1" | "D3" | "D5");
    let on = if high_side { tau < 0.46 } else { tau > 0.54 };
    if on {
        GateState::On
    } else {
        GateState::Off
    }
}

fn run(dt: f64) -> Vec<(f64, OperatingPoint)> {
    let switch = IdealSwitch::new(R_ON, IdealDiode::new(1e-9, -2000.0, 1e-9, V_F, G_ON));
    let ideal_switches: BTreeMap<String, IdealSwitch> = ["D1", "D2", "D3", "D4", "D5", "D6"]
        .iter()
        .map(|name| (name.to_string(), switch))
        .collect();
    simulate_transient_with_ideal_switches(
        NETLIST,
        Dialect::Ngspice,
        &BTreeMap::new(),
        &ideal_switches,
        gate_state,
        R_ON,
        None,
        T_FINAL,
        dt,
    )
    .expect("the three-leg bridge solves at every step size")
}

/// `(lowest, highest)` voltage node `node` reaches anywhere in `trace`.
fn node_extremes(trace: &[(f64, OperatingPoint)], node: &str) -> (f64, f64) {
    let name = format!("V({node})");
    trace.iter().fold((f64::MAX, f64::MIN), |(lo, hi), (_, p)| {
        let v = p.value(&name).expect("the netlist declares this node");
        (lo.min(v), hi.max(v))
    })
}

/// The largest magnitude `unknown` reaches anywhere in `trace`.
fn peak_magnitude(trace: &[(f64, OperatingPoint)], unknown: &str) -> f64 {
    trace.iter().fold(0.0f64, |peak, (_, p)| {
        peak.max(
            p.value(unknown)
                .expect("the netlist declares this branch")
                .abs(),
        )
    })
}

#[test]
fn a_switching_node_stays_inside_its_body_diode_clamp_at_every_step_size() {
    // 5 ns, 0.5 ns, 0.05 ns: the same three step sizes the report compared, an order of
    // magnitude apart each time.
    let step_sizes = [5e-9, 5e-10, 5e-11];
    let mut extremes_per_step: Vec<[(f64, f64); 3]> = Vec::new();
    // Collected rather than asserted one at a time: which leg misbehaves depends on the step
    // size, so a failure that stops at the first one hides that the other step sizes are
    // equally wrong, in a different leg.
    let mut violations: Vec<String> = Vec::new();

    for dt in step_sizes {
        let trace = run(dt);
        let mut extremes = [(0.0, 0.0); 3];
        for (leg, (node, inductor)) in LEGS.iter().enumerate() {
            let load_current = peak_magnitude(&trace, inductor);
            let drop = V_F + load_current / G_ON + CLAMP_TOLERANCE;
            let (lo, hi) = node_extremes(&trace, node);
            extremes[leg] = (lo, hi);
            if hi > V_RAIL + drop {
                violations.push(format!(
                    "dt={dt:e}: V({node}) reached {hi:.2} V, above the {:.2} V its high-side \
                     body diode clamps it to while freewheeling {load_current:.1} A",
                    V_RAIL + drop
                ));
            }
            if lo < -drop {
                violations.push(format!(
                    "dt={dt:e}: V({node}) reached {lo:.2} V, below the {:.2} V its low-side \
                     body diode clamps it to while freewheeling {load_current:.1} A",
                    -drop
                ));
            }
        }
        extremes_per_step.push(extremes);
    }

    // The clamp belongs to the device, so the same node's extremes must not depend on the step
    // size either — the property the overshoot violated most visibly.
    for (leg, (node, _)) in LEGS.iter().enumerate() {
        for (bound, label) in [(0usize, "lowest"), (1, "highest")] {
            let values: Vec<f64> = extremes_per_step
                .iter()
                .map(|e| if bound == 0 { e[leg].0 } else { e[leg].1 })
                .collect();
            let spread = values.iter().cloned().fold(f64::MIN, f64::max)
                - values.iter().cloned().fold(f64::MAX, f64::min);
            if spread > STEP_INDEPENDENCE_TOLERANCE {
                violations.push(format!(
                    "the {label} V({node}) is {values:?} at {step_sizes:?}: a spread of \
                     {spread:.2} V, which a device clamp cannot have"
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "a switching node left its own body diodes' clamp window:\n  {}",
        violations.join("\n  ")
    );
}
