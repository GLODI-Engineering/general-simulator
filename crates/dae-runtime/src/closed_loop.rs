//! Wires a `continuous_blocks::StateSpace` controller (typically a compiled [`Pid`
//! ](continuous_blocks::Pid)) into a closed loop around a circuit's MOSFET(s), replacing
//! Xyce/SPICE's `tanh`-smoothed comparator workaround
//! (`internal-archive`'s `gotchas/xyce-pid-timestep-collapse-needs-smooth-comparator.md`)
//! with an ordinary PWM comparator — no smoothing hack needed, since gate switching is just
//! another LCP-resolved mode here, not a Newton-Raphson convergence hazard. See
//! `docs/architecture.md`.
//!
//! This is deliberately a **sampled-data co-simulation**, not a fully implicit unified system:
//! the controller reads the circuit's measured output from the *previous* timestep, steps its
//! own dynamics forward with its own RK4 integrator, and the result (compared against a PWM
//! carrier) decides this step's gate states before the circuit step is solved. This is not a
//! shortcut — it is literally how a real digital PID+PWM controller in an actual converter
//! works: it samples state, computes, and updates PWM at discrete instants. Fully folding a
//! controller's `(A, K, B)` into the circuit's own global descriptor system (implicit,
//! zero-sample-delay coupling) remains a possible future refinement, not a correctness gap in
//! what's implemented here.

use std::collections::BTreeMap;

use continuous_blocks::StateSpace;
use pwl_devices::{Diode, Mosfet};
use spice_core::Dialect;

use crate::{classify_segments, step_with_fallback, DaeError, GateState, OperatingPoint, Segment};

/// A free-running sawtooth PWM carrier in `[0, 1)` at frequency `freq_hz`. Compare a duty
/// command (also `[0, 1]`) against this to decide gate state: on while the carrier is below
/// the duty command, off otherwise (a standard trailing-edge PWM comparator).
pub fn sawtooth_carrier(t: f64, freq_hz: f64) -> f64 {
    let phase = t * freq_hz;
    phase - phase.floor()
}

/// Runs a closed-loop transient: at each timestep, `measure` reads a scalar from the
/// *previous* step's [`OperatingPoint`] (e.g. an output node voltage), `controller` (its own
/// state carried forward internally) is stepped with `reference - measured` as its input via
/// RK4, and `pwm(controller_output, t)` decides every MOSFET's gate state for this step from
/// that controller output. Everything else matches [`crate::simulate_transient_with_mosfets`]
/// (system rebuilt per step, trapezoidal with backward-Euler fallback on any diode-segment or
/// gate-state change).
///
/// Returns, per step: `(t, OperatingPoint, controller_output)`.
#[allow(clippy::too_many_arguments)]
pub fn simulate_closed_loop(
    source: &str,
    dialect: Dialect,
    diodes: &BTreeMap<String, Diode>,
    mosfets: &BTreeMap<String, Mosfet>,
    controller: &StateSpace,
    reference: f64,
    measure: impl Fn(&OperatingPoint) -> f64,
    pwm: impl Fn(f64, f64) -> BTreeMap<String, GateState>,
    shared_r_on: f64,
    x_initial: Option<&[f64]>,
    t_final: f64,
    dt: f64,
) -> Result<Vec<(f64, OperatingPoint, f64)>, DaeError> {
    // A zero initial gate assignment (no measurement exists yet) just to learn `order` for the
    // default x_initial and to get a first OperatingPoint to seed `measure`/the loop with.
    let initial_states: BTreeMap<String, (Mosfet, GateState)> = mosfets
        .iter()
        .map(|(name, m)| (name.clone(), (*m, GateState::Off)))
        .collect();
    let (system0, all_diodes0) =
        crate::build_with_mosfets(source, dialect, diodes, &initial_states, shared_r_on)?;
    let mut x_prev = match x_initial {
        Some(x) => x.to_vec(),
        None => vec![0.0; system0.order()],
    };
    let mut point_prev = step_with_fallback(
        &system0,
        source,
        dialect,
        &all_diodes0,
        &x_prev,
        dt,
        &BTreeMap::new(),
        &None,
        true,
    )?;
    x_prev = point_prev.x.clone();

    let mut controller_x = vec![0.0; controller.states()];
    let mut prev_diode_raw_ioff: BTreeMap<String, f64> = BTreeMap::new();
    let mut prev_segments: Option<Vec<Segment>> = None;
    let mut prev_gate_states: Option<BTreeMap<String, GateState>> = None;

    let steps = (t_final / dt).round() as usize;
    let mut trace = Vec::with_capacity(steps);
    let mut t = 0.0;
    for step_index in 0..steps {
        t += dt;

        let measured = measure(&point_prev);
        let error = [reference - measured];
        controller_x = controller.rk4_step(&controller_x, &error, dt);
        let controller_output = controller.output(&controller_x, &error)[0];

        let gate_states = pwm(controller_output, t);
        let states: BTreeMap<String, (Mosfet, GateState)> = mosfets
            .iter()
            .map(|(name, m)| {
                (
                    name.clone(),
                    (*m, *gate_states.get(name).unwrap_or(&GateState::Off)),
                )
            })
            .collect();
        let gate_changed = prev_gate_states.as_ref() != Some(&gate_states);

        let (system, all_diodes) =
            crate::build_with_mosfets(source, dialect, diodes, &states, shared_r_on)?;
        let point = step_with_fallback(
            &system,
            source,
            dialect,
            &all_diodes,
            &x_prev,
            dt,
            &prev_diode_raw_ioff,
            &prev_segments,
            step_index == 0 || gate_changed,
        )?;

        prev_segments = Some(classify_segments(&point));
        prev_gate_states = Some(gate_states);
        prev_diode_raw_ioff = point
            .diode_names
            .iter()
            .cloned()
            .zip(point.diode_raw_ioff.iter().copied())
            .collect();
        x_prev = point.x.clone();
        point_prev = point.clone();
        trace.push((t, point, controller_output));
    }
    Ok(trace)
}
