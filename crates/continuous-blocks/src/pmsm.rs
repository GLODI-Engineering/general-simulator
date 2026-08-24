//! A permanent-magnet synchronous motor (PMSM), in the rotor `d`/`q` reference frame — the
//! standard textbook electromechanical model, built to close the loop around
//! `coordinate_transforms`' Clarke/Park blocks for a field-oriented-control (FOC) motor-drive
//! experiment: an inverter's three-phase output feeds this block (through `clarke_park`), an
//! outer speed loop and inner `id`/`iq` current loops regulate it, and `clarke_park_inv` turns
//! the resulting `d`/`q` voltage commands back into three-phase PWM references.
//!
//! Deliberately not a [`crate::StateSpace`]: the back-EMF/cross-coupling terms (`omega_e * l_q *
//! iq`, `omega_e * l_d * id`) and the electromagnetic torque (`(l_d - l_q) * id * iq`) are
//! *bilinear* — products of two of this block's own states (speed and current), not linear in
//! the state vector — so this genuinely cannot be expressed as the descriptor system `A x + K
//! dx/dt = B u` every other dynamic block in this crate compiles to. Same category as
//! [`crate::Vco`]: a small, self-contained block with its own bespoke `step()`, integrated
//! explicitly (here, RK4 over its own 4-state nonlinear vector field) rather than through the
//! generic `StateSpace` machinery. Because the whole nonlinear vector field is evaluated
//! *inside* `step()`, RK4's own intermediate stages naturally handle the bilinear coupling
//! correctly (each stage evaluates the same [`Pmsm::derivative`] at its own trial state) — there
//! is no cross-block algebraic loop to resolve the way there would be if `id`/`iq` dynamics were
//! split across two separate blocks that each needed the other's current-step output.
//!
//! State is `(id, iq, omega_m, theta_e)`: stator current `d`/`q` components (A), mechanical
//! speed (rad/s), and *unwrapped* electrical angle (rad) — kept unwrapped through the
//! integration (so RK4 never has to reason about a discontinuity mid-step) and wrapped to `[0,
//! 2*pi)` only by [`Pmsm::theta_e_wrapped`] for a caller that needs it as an input to
//! [`crate::coordinate_transforms::park`]/[`crate::coordinate_transforms::clarke_park`].
//! Electrical angular speed is `omega_e = pole_pairs * omega_m`; `theta_e` does not itself feed
//! back into the `id`/`iq`/`omega_m` dynamics, only accumulates `omega_e`.

use std::f64::consts::PI;

/// Motor parameters (SI units): stator resistance `r_s` (ohm), `d`/`q`-axis inductance `l_d`/
/// `l_q` (H — equal for a surface-mount PMSM, unequal for an interior-PM motor with reluctance
/// torque), permanent-magnet flux linkage `lambda_pm` (Wb), `pole_pairs` (electrical cycles per
/// mechanical revolution), rotor+load inertia `inertia` (kg*m^2), and viscous friction
/// coefficient `friction` (N*m*s/rad).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pmsm {
    pub r_s: f64,
    pub l_d: f64,
    pub l_q: f64,
    pub lambda_pm: f64,
    pub pole_pairs: f64,
    pub inertia: f64,
    pub friction: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PmsmError {
    NegativeResistance,
    NonPositiveInductance,
    NonPositivePolePairs,
    NonPositiveInertia,
    NegativeFriction,
}

impl Pmsm {
    pub fn new(
        r_s: f64,
        l_d: f64,
        l_q: f64,
        lambda_pm: f64,
        pole_pairs: f64,
        inertia: f64,
        friction: f64,
    ) -> Result<Self, PmsmError> {
        if r_s < 0.0 {
            return Err(PmsmError::NegativeResistance);
        }
        if !(l_d > 0.0 && l_q > 0.0) {
            return Err(PmsmError::NonPositiveInductance);
        }
        if pole_pairs <= 0.0 {
            return Err(PmsmError::NonPositivePolePairs);
        }
        if inertia <= 0.0 {
            return Err(PmsmError::NonPositiveInertia);
        }
        if friction < 0.0 {
            return Err(PmsmError::NegativeFriction);
        }
        Ok(Pmsm {
            r_s,
            l_d,
            l_q,
            lambda_pm,
            pole_pairs,
            inertia,
            friction,
        })
    }

    /// Electromagnetic torque (N*m) at a given `id`/`iq` operating point — the standard
    /// permanent-magnet-alignment term plus reluctance torque (zero when `l_d == l_q`, the
    /// surface-mount case).
    pub fn torque(&self, id: f64, iq: f64) -> f64 {
        1.5 * self.pole_pairs * (self.lambda_pm * iq + (self.l_d - self.l_q) * id * iq)
    }

    /// This block's own electrical angle, wrapped to `[0, 2*pi)` — matches
    /// [`crate::coordinate_transforms::anglewrap`]'s own convention, so a downstream `Park`/
    /// `ClarkePark` block can consume either interchangeably.
    pub fn theta_e_wrapped(theta_e: f64) -> f64 {
        theta_e.rem_euclid(2.0 * PI)
    }

    /// The nonlinear vector field `dx/dt = f(x, vd, vq, t_load)`, `x = (id, iq, omega_m,
    /// theta_e)`. `vd`/`vq`/`t_load` are held fixed across a call — [`Pmsm::step`]'s RK4 stages
    /// evaluate this at several trial states within one timestep, not several trial inputs.
    fn derivative(&self, x: [f64; 4], vd: f64, vq: f64, t_load: f64) -> [f64; 4] {
        let [id, iq, omega_m, _theta_e] = x;
        let omega_e = self.pole_pairs * omega_m;
        let did = (vd - self.r_s * id + omega_e * self.l_q * iq) / self.l_d;
        let diq =
            (vq - self.r_s * iq - omega_e * self.l_d * id - omega_e * self.lambda_pm) / self.l_q;
        let domega = (self.torque(id, iq) - t_load - self.friction * omega_m) / self.inertia;
        let dtheta = omega_e;
        [did, diq, domega, dtheta]
    }

    /// Advances the motor state by one step via classical 4th-order Runge-Kutta, with `vd`/
    /// `vq`/`t_load` held fixed (zero-order hold) across the step — the same convention
    /// [`crate::StateSpace::rk4_step`] uses for its own single-step input.
    pub fn step(&self, x: [f64; 4], vd: f64, vq: f64, t_load: f64, dt: f64) -> [f64; 4] {
        let f = |s: [f64; 4]| self.derivative(s, vd, vq, t_load);
        let add = |a: [f64; 4], b: [f64; 4], s: f64| -> [f64; 4] {
            std::array::from_fn(|i| a[i] + s * b[i])
        };

        let k1 = f(x);
        let k2 = f(add(x, k1, dt / 2.0));
        let k3 = f(add(x, k2, dt / 2.0));
        let k4 = f(add(x, k3, dt));

        std::array::from_fn(|i| x[i] + (dt / 6.0) * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn torque_matches_hand_derived_formula_including_reluctance_term() {
        let motor = Pmsm::new(1.0, 2e-3, 3e-3, 0.05, 4.0, 1e-4, 0.0).unwrap();
        // Te = 1.5 * p * (lambda_pm*iq + (Ld-Lq)*id*iq)
        //    = 1.5 * 4 * (0.05*2.0 + (2e-3-3e-3)*1.5*2.0)
        //    = 6 * (0.1 - 0.003) = 6 * 0.097 = 0.582
        let te = motor.torque(1.5, 2.0);
        assert!((te - 0.582).abs() < 1e-12, "Te={te}");

        // Surface-mount case (Ld=Lq): reluctance term vanishes, torque is purely
        // current-proportional.
        let surface = Pmsm::new(1.0, 2e-3, 2e-3, 0.05, 4.0, 1e-4, 0.0).unwrap();
        let te_surface = surface.torque(1.5, 2.0);
        assert!((te_surface - 1.5 * 4.0 * 0.05 * 2.0).abs() < 1e-12);
    }

    /// With `iq(0) = 0` and `vq = 0`, both `iq` and `omega_m` stay identically zero for all
    /// time (`diq/dt` and `Te` both vanish whenever `iq = 0`, `domega/dt` then has no torque
    /// input and starts at `omega_m = 0`), which decouples the `d`-axis into an ordinary
    /// R-L circuit driven by `vd` — a case with an exact closed-form solution,
    /// `id(t) = (vd/R)*(1 - exp(-t*R/Ld))`, independent of this model's own RK4 implementation.
    /// This is therefore a genuine hand-derived check on the electrical dynamics, not just
    /// internal self-consistency.
    #[test]
    fn d_axis_step_response_matches_hand_derived_rl_circuit_when_decoupled() {
        let motor = Pmsm::new(2.0, 5e-3, 5e-3, 0.05, 4.0, 1e-4, 0.0).unwrap();
        let (vd, r, l) = (10.0, 2.0, 5e-3);
        let tau = l / r;

        let mut x = [0.0, 0.0, 0.0, 0.0];
        let dt = tau / 200.0; // fine steps relative to the R-L time constant
        let mut t = 0.0;
        for _ in 0..400 {
            x = motor.step(x, vd, 0.0, 0.0, dt);
            t += dt;
        }
        let expected_id = (vd / r) * (1.0 - (-t * r / l).exp());
        assert!(
            (x[0] - expected_id).abs() < 1e-6,
            "id={}, expected={}",
            x[0],
            expected_id
        );
        assert_eq!(x[1], 0.0, "iq must stay exactly zero when decoupled");
        assert_eq!(x[2], 0.0, "omega_m must stay exactly zero when decoupled");
    }

    /// Convergence-order check on the full coupled nonlinear system (nonzero `vd`, `vq`,
    /// `t_load`, and a nonzero initial speed, so every bilinear coupling term is actually
    /// engaged) — the same technique `dae-runtime`'s own transient loop is verified with
    /// (halving `dt` and checking the error shrinks at RK4's own 4th-order rate against a
    /// converged reference), the standard way to validate a numerical integrator on a system
    /// with no closed-form solution.
    #[test]
    fn full_coupled_system_converges_at_fourth_order() {
        let motor = Pmsm::new(1.0, 3e-3, 5e-3, 0.08, 4.0, 2e-4, 1e-4).unwrap();
        let x0 = [0.0, 0.0, 50.0, 0.0];
        let (vd, vq, t_load) = (20.0, 40.0, 0.02);
        let t_final = 2e-3;

        let run = |steps: usize| -> [f64; 4] {
            let dt = t_final / steps as f64;
            let mut x = x0;
            for _ in 0..steps {
                x = motor.step(x, vd, vq, t_load, dt);
            }
            x
        };

        let reference = run(4096); // fine enough to stand in for the true solution
        let coarse = run(64);
        let fine = run(128);

        let err = |a: [f64; 4], b: [f64; 4]| -> f64 {
            a.iter()
                .zip(b.iter())
                .map(|(x, y)| (x - y).powi(2))
                .sum::<f64>()
                .sqrt()
        };
        let err_coarse = err(coarse, reference);
        let err_fine = err(fine, reference);

        // Halving dt should shrink RK4's error by roughly 2^4 = 16.
        let ratio = err_coarse / err_fine;
        assert!(
            (8.0..32.0).contains(&ratio),
            "expected roughly 16x error reduction halving dt, got ratio={ratio} \
             (err_coarse={err_coarse}, err_fine={err_fine})"
        );
    }

    #[test]
    fn theta_e_wraps_to_0_2pi_matching_anglewrap_convention() {
        assert!((Pmsm::theta_e_wrapped(0.5) - 0.5).abs() < 1e-12);
        assert!((Pmsm::theta_e_wrapped(2.0 * PI + 0.5) - 0.5).abs() < 1e-12);
        assert!((Pmsm::theta_e_wrapped(-0.5) - (2.0 * PI - 0.5)).abs() < 1e-12);
    }

    #[test]
    fn invalid_parameters_are_rejected_not_a_panic() {
        assert_eq!(
            Pmsm::new(-1.0, 2e-3, 2e-3, 0.05, 4.0, 1e-4, 0.0),
            Err(PmsmError::NegativeResistance)
        );
        assert_eq!(
            Pmsm::new(1.0, 0.0, 2e-3, 0.05, 4.0, 1e-4, 0.0),
            Err(PmsmError::NonPositiveInductance)
        );
        assert_eq!(
            Pmsm::new(1.0, 2e-3, 2e-3, 0.05, 0.0, 1e-4, 0.0),
            Err(PmsmError::NonPositivePolePairs)
        );
        assert_eq!(
            Pmsm::new(1.0, 2e-3, 2e-3, 0.05, 4.0, 0.0, 0.0),
            Err(PmsmError::NonPositiveInertia)
        );
        assert_eq!(
            Pmsm::new(1.0, 2e-3, 2e-3, 0.05, 4.0, 1e-4, -1.0),
            Err(PmsmError::NegativeFriction)
        );
    }
}
