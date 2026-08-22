//! How a transient run's timestep is chosen: a fixed, caller-given `dt` throughout (simple,
//! deterministic, exactly reproducible — what every function in this crate did until this
//! module existed), or local-truncation-error-driven adaptive step-size control (small steps
//! automatically where the solution is changing fast, large steps where it's settled or
//! between switching events — what every real SPICE-family tool does by default, because a
//! circuit's dynamics are rarely uniform in speed across a whole run: see [`AdaptiveConfig`]
//! and [`lte_attempt`] for the actual algorithm).
//!
//! [`TimeStep::Fixed`] keeps exactly the original behavior (used directly, no LTE machinery
//! touched at all) — this module doesn't change anything about the fixed-step path beyond
//! giving it a name in this enum.

use std::collections::BTreeMap;

use elspice_mna::MnaSystem;
use pwl_devices::Diode;
use spice_core::Dialect;

use crate::{
    classify_segments, fold_and_solve, is_ringing, DaeError, OperatingPoint, Scheme, Segment,
};

/// How a transient run picks its step size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TimeStep {
    /// The same `dt` every step, exactly as before this module existed.
    Fixed(f64),
    /// Local-truncation-error-driven: see [`AdaptiveConfig`].
    Adaptive(AdaptiveConfig),
}

/// Parameters for [`lte_attempt`]'s local-truncation-error step-size control.
///
/// `reltol`/`abstol` together form the per-unknown error scale `abstol + reltol *
/// max(|x_trapezoidal|, |x_backward_euler|)` a step's estimated error is normalized against
/// (the same `RELTOL`/`ABSTOL` idea every SPICE-family tool exposes) — `reltol` alone isn't
/// enough because it degenerates to zero right where many circuit unknowns spend a lot of
/// their time (an AC-coupled voltage or an inductor current crossing zero), so `abstol` is the
/// floor that keeps step control meaningful there. This crate uses one scalar `abstol` for
/// every unknown regardless of whether it's a voltage or a current — a real simplification
/// relative to SPICE's own separate voltage/current tolerances (`VNTOL`/`ABSTOL`), noted
/// directly rather than silently: pick `abstol` relative to the smaller-magnitude unknowns in
/// your own circuit if the defaults reject too aggressively.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AdaptiveConfig {
    /// The very first trial step size.
    pub dt_init: f64,
    /// Never shrink below this, no matter how large the estimated error — accept whatever
    /// result comes out instead of looping forever (a run that only converges below `dt_min`
    /// needs a smaller `dt_min` or looser tolerances, not an infinite retry loop).
    pub dt_min: f64,
    /// Never grow above this, even if the estimated error says a step could be much larger —
    /// the same role `TSTEP`/a `.tran` step ceiling plays in a real SPICE deck: without it, a
    /// long quiet stretch could grow the step past the resolution needed to catch the *next*
    /// fast event at all.
    pub dt_max: f64,
    /// Relative error tolerance (dimensionless, e.g. `1e-3` for 0.1%).
    pub reltol: f64,
    /// Absolute error tolerance (same units as the circuit's own unknowns — volts/amps).
    pub abstol: f64,
}

impl AdaptiveConfig {
    /// A reasonable default derived from `t_final` alone, for when the caller hasn't measured
    /// their own circuit's time constants — the same situation a real SPICE `.tran` line is
    /// in when only `TSTOP` is meaningfully chosen by the user and the internal step is left to
    /// the tool. `dt_max = t_final/1000` (a 1000-point default resolution, comparable to a
    /// typical SPICE run's point count), `dt_init = dt_max/100` (start conservatively, like
    /// every real tool's own small first step, then grow once the solution's behavior is
    /// actually known), `dt_min = dt_max*1e-9`.
    pub fn from_t_final(t_final: f64) -> Self {
        let dt_max = t_final / 1000.0;
        AdaptiveConfig {
            dt_init: dt_max / 100.0,
            dt_min: dt_max * 1e-9,
            dt_max,
            reltol: 1e-3,
            abstol: 1e-9,
        }
    }
}

/// The result of one [`lte_attempt`] at one candidate `dt` — the caller decides accept/reject
/// (`accept`) and the next `dt` to try (`suggested_dt_next`, already the right value for
/// *either* outcome: use it as-is on accept, or as the next attempt's `dt` on reject).
pub(crate) struct LteAttempt {
    pub point: OperatingPoint,
    pub used_backward_euler: bool,
    pub accept: bool,
    pub suggested_dt_next: f64,
}

/// One local-truncation-error attempt at a candidate `dt`, against an already-built `system`
/// (the caller is responsible for rebuilding `system` at this `dt`/gate-state combination
/// first, when that matters — e.g. a MOSFET circuit whose gate states are themselves a
/// function of `dt` through a block graph; see `block_graph`'s own adaptive loop).
///
/// **The error estimate**: backward Euler (order 1, `O(h^2)` local error) and trapezoidal
/// (order 2, `O(h^3)` local error) are both solved at the *same* trial `dt`; their difference
/// is dominated by backward Euler's own larger error term, giving a cheap, history-free proxy
/// for trapezoidal's actual error — the same idea an embedded Runge-Kutta pair (e.g. Runge-
/// Kutta-Fehlberg) uses two different-order formulas for, adapted here to the two schemes this
/// crate already implements rather than introducing new ones. The accepted result is always
/// the (more accurate) trapezoidal one, never backward Euler, except when a segment/gate
/// change or ringing is detected (same as [`crate::step_with_fallback`]) or when the caller
/// already knows this step must be backward Euler (`force_backward_euler` — the first step of
/// a run, right after a gate change, or during a ringing cooldown; see
/// [`crate::step_with_fallback`]'s own doc comment for why those specific cases need it).
///
/// Step-size update, on acceptance: `dt_next = dt * clamp(sqrt(1/err), 0.2, 5.0) * 0.9`, the
/// `0.9` a standard safety margin so a step doesn't repeatedly land right at the tolerance
/// boundary and oscillate between barely-accepted and rejected. On rejection: `dt_next = dt *
/// clamp(sqrt(1/err), 0.1, 0.5)` (always shrinks). After a forced/discontinuity step,
/// `suggested_dt_next` is conservatively halved rather than left to the error estimate — the
/// same reason [`crate::step_with_fallback`]'s ringing-cooldown exists: trust the solution
/// less immediately after a mode change, not more.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lte_attempt(
    system: &MnaSystem,
    source: &str,
    dialect: Dialect,
    diodes: &BTreeMap<String, Diode>,
    x_prev_prev: Option<&[f64]>,
    x_prev: &[f64],
    dt: f64,
    prev_diode_raw_ioff: &BTreeMap<String, f64>,
    prev_segments: &Option<Vec<Segment>>,
    force_backward_euler: bool,
    config: &AdaptiveConfig,
    t_start: f64,
    extra_values: &BTreeMap<String, f64>,
    extra_values_prev: &BTreeMap<String, f64>,
) -> Result<LteAttempt, DaeError> {
    let t = t_start + dt;
    if force_backward_euler {
        let point = fold_and_solve(
            system,
            source,
            dialect,
            diodes,
            &Scheme::BackwardEuler { x_prev, dt },
            t,
            extra_values,
            extra_values_prev,
        )?;
        let suggested_dt_next = (dt * 0.5).clamp(config.dt_min, config.dt_max);
        return Ok(LteAttempt {
            point,
            used_backward_euler: true,
            accept: true,
            suggested_dt_next,
        });
    }

    let be = fold_and_solve(
        system,
        source,
        dialect,
        diodes,
        &Scheme::BackwardEuler { x_prev, dt },
        t,
        extra_values,
        extra_values_prev,
    )?;
    let trap = fold_and_solve(
        system,
        source,
        dialect,
        diodes,
        &Scheme::Trapezoidal {
            x_prev,
            dt,
            prev_diode_raw_ioff,
        },
        t,
        extra_values,
        extra_values_prev,
    )?;

    let trial_segments = classify_segments(&trap);
    let segments_changed = Some(&trial_segments) != prev_segments.as_ref();
    let ringing = x_prev_prev.is_some_and(|xpp| is_ringing(xpp, x_prev, &trap.x));
    if segments_changed || ringing {
        // Same policy as step_with_fallback: don't trust trapezoidal across a resolved mode
        // change or detected ringing -- take backward Euler outright, and restart the growth
        // ramp conservatively rather than trusting an error estimate computed against a
        // trapezoidal result that's being discarded anyway.
        let suggested_dt_next = (dt * 0.5).clamp(config.dt_min, config.dt_max);
        return Ok(LteAttempt {
            point: be,
            used_backward_euler: true,
            accept: true,
            suggested_dt_next,
        });
    }

    let err = trap.x.iter().zip(&be.x).fold(0.0_f64, |acc, (&xt, &xb)| {
        let scale = config.abstol + config.reltol * xt.abs().max(xb.abs());
        acc.max((xt - xb).abs() / scale)
    });

    let at_floor = dt <= config.dt_min * (1.0 + 1e-9);
    if err <= 1.0 || at_floor {
        let growth = if err > 1e-12 { (1.0 / err).sqrt() } else { 5.0 };
        let suggested_dt_next =
            (dt * (growth.clamp(0.2, 5.0)) * 0.9).clamp(config.dt_min, config.dt_max);
        Ok(LteAttempt {
            point: trap,
            used_backward_euler: false,
            accept: true,
            suggested_dt_next,
        })
    } else {
        let shrink = (1.0 / err).sqrt().clamp(0.1, 0.5);
        let suggested_dt_next = (dt * shrink).max(config.dt_min);
        Ok(LteAttempt {
            point: trap,
            used_backward_euler: false,
            accept: false,
            suggested_dt_next,
        })
    }
}

/// The common case built on [`lte_attempt`]: `system` is fixed for the whole run (no MOSFETs,
/// or any other reason gate/topology doesn't depend on `dt`), so this owns the retry loop
/// directly. `block_graph`'s adaptive loop can't use this — its `system` is rebuilt from block
/// outputs that are themselves a function of `dt`, so it drives [`lte_attempt`] with its own
/// retry loop instead (see that module).
#[allow(clippy::too_many_arguments)]
pub(crate) fn adaptive_step(
    system: &MnaSystem,
    source: &str,
    dialect: Dialect,
    diodes: &BTreeMap<String, Diode>,
    x_prev_prev: Option<&[f64]>,
    x_prev: &[f64],
    dt_trial: f64,
    prev_diode_raw_ioff: &BTreeMap<String, f64>,
    prev_segments: &Option<Vec<Segment>>,
    force_backward_euler: bool,
    config: &AdaptiveConfig,
    t_start: f64,
) -> Result<(OperatingPoint, bool, f64, f64), DaeError> {
    let mut dt = dt_trial.clamp(config.dt_min, config.dt_max);
    let empty = BTreeMap::new();
    loop {
        let attempt = lte_attempt(
            system,
            source,
            dialect,
            diodes,
            x_prev_prev,
            x_prev,
            dt,
            prev_diode_raw_ioff,
            prev_segments,
            force_backward_euler,
            config,
            t_start,
            &empty,
            &empty,
        )?;
        if attempt.accept {
            return Ok((
                attempt.point,
                attempt.used_backward_euler,
                dt,
                attempt.suggested_dt_next,
            ));
        }
        dt = attempt.suggested_dt_next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_t_final_orders_the_three_step_sizes_sensibly() {
        let config = AdaptiveConfig::from_t_final(1e-3);
        assert!(config.dt_min < config.dt_init);
        assert!(config.dt_init < config.dt_max);
        assert_eq!(config.dt_max, 1e-6);
    }
}
