//! Assembles a circuit's linear MNA system (via `elspice-mna`, including its `'D'` stamp) with
//! PWL device segments (via `pwl-devices`) into one Linear Complementarity Problem, solves it
//! (via `lcp-solver`), and reports the resolved DC operating point. No Newton-Raphson, no
//! voltage limiting, anywhere in this crate — see this repository's `docs/architecture.md`.
//!
//! ## The generic Thevenin/LCP fold
//!
//! `elspice-mna` stamps each diode `k` as a *fixed* conductance `{k}_G` plus a per-instance
//! current-source symbol `{k}_Ioff`. Fixing `{k}_G` at the diode's canonical reference slope
//! `g_off` (see `pwl_devices::Diode::canonical`) makes the whole linear system `A0` genuinely
//! fixed — independent of which segment every diode ends up in — with every segment's actual
//! nonlinearity pushed entirely into the `{k}_Ioff` terms via each diode's canonical
//! `max(0, ...)` (`z`) decomposition. Since the system is linear in those `Ioff` terms, each
//! diode's terminal voltage is an affine function of every diode's `z` variables:
//!
//! ```text
//! x(z)   = x0 + sum_j w_j * raw_Ioff_j                  (raw_Ioff_j = delta_on_j*z2_j - delta_br_j*z1_j)
//! V_k(z) = v0_k + sum_j gamma_kj * raw_Ioff_j            (gamma_kj = w_j[p_k] - w_j[n_k])
//! ```
//!
//! where `x0 = solve(A0, u0)` is the baseline operating point with every diode's `Ioff = 0`,
//! and `w_j = solve(A0, B[:, diode j's input column])` is how much a unit "raw Ioff" on diode
//! `j` moves every unknown. Substituting `V_k(z)` into each diode's guard equations
//! (`w1_k = V_k - v_breakdown_k + z1_k`, `w2_k = v_th_k - V_k + z2_k`) gives exactly the LCP
//! `(M, q)` this crate builds and hands to `lcp_solver::solve`.

mod linsolve;
mod topology;

use std::collections::BTreeMap;

use elspice_mna::{BuildError, BuildOptions, EvaluationError, Expression, MnaBuilder, MnaSystem};
use lcp_solver::LcpError;
use linsolve::{dense_solve, SingularMatrix};
use pwl_devices::{Diode, Mosfet};
use spice_core::Dialect;

pub use elspice_mna::SwitchState as GateState;

#[derive(Debug, Clone, PartialEq)]
pub struct OperatingPoint {
    pub unknowns: Vec<String>,
    pub x: Vec<f64>,
    /// Resolved `(z1, z2)` for each diode, in the same iteration order as the `diodes` map
    /// passed to [`solve_dc`] (a `BTreeMap`, so alphabetical by name) — useful for reporting
    /// which segment each diode ended up in.
    pub diode_names: Vec<String>,
    pub diode_z: Vec<(f64, f64)>,
}

impl OperatingPoint {
    pub fn value(&self, unknown: &str) -> Option<f64> {
        self.unknowns
            .iter()
            .position(|name| name == unknown)
            .map(|index| self.x[index])
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DaeError {
    Build(BuildError),
    Evaluate(EvaluationError),
    Linear(SingularMatrix),
    Lcp(LcpError),
    UnknownDiodeInput(String),
}

/// Solves the DC operating point of a netlist containing linear devices plus any number of
/// `D` (diode) elements, whose piecewise-linear parameters are supplied in `diodes` (keyed by
/// element name) rather than parsed from the netlist — SPICE's `.model` syntax has no notion
/// of this crate's breakdown/leakage/forward segments.
pub fn solve_dc(
    source: &str,
    dialect: Dialect,
    diodes: &BTreeMap<String, Diode>,
) -> Result<OperatingPoint, DaeError> {
    let system = MnaBuilder::new(dialect)
        .build_fragment(source)
        .map_err(DaeError::Build)?;
    fold_and_solve(system, source, dialect, diodes)
}

/// Solves the DC operating point of a netlist containing linear devices, ordinary `D` diodes
/// (`diodes`), and any number of `Mosfet` instances (`mosfets`), each with its own known gate
/// state (see [`GateState`], a re-export of `elspice_mna::SwitchState` — the same concept: an
/// exogenous, externally-decided mode, not something the LCP resolves).
///
/// A gated-on MOSFET is stamped as a plain `r_on` switch (reusing `elspice-mna`'s existing
/// switch mechanism — every gated-on MOSFET in one call shares `shared_r_on`, matching that
/// mechanism's own single-shared-resistance design; per-instance `Ron` is a possible future
/// extension, not needed yet). A gated-off MOSFET is folded into the LCP exactly like an
/// ordinary diode, using its `body_diode` — see [`Mosfet`]'s doc comment for the
/// `(source, drain)` node-order convention this requires in the netlist. Each MOSFET element
/// must still use device letter `'D'` in the netlist text (not `'M'`) — see this crate's
/// `docs`/journal for why.
pub fn solve_dc_with_mosfets(
    source: &str,
    dialect: Dialect,
    diodes: &BTreeMap<String, Diode>,
    mosfets: &BTreeMap<String, (Mosfet, GateState)>,
    shared_r_on: f64,
) -> Result<OperatingPoint, DaeError> {
    let mut options = BuildOptions {
        on_resistance: Expression::Constant(shared_r_on),
        ..BuildOptions::default()
    };

    let mut all_diodes = diodes.clone();
    for (name, (mosfet, state)) in mosfets {
        match state {
            GateState::On => options.set_switch(name, GateState::On),
            GateState::Off => {
                all_diodes.insert(name.clone(), mosfet.body_diode);
            }
        }
    }

    let system = MnaBuilder::with_options(dialect, options)
        .build_fragment(source)
        .map_err(DaeError::Build)?;
    fold_and_solve(system, source, dialect, &all_diodes)
}

fn fold_and_solve(
    system: MnaSystem,
    source: &str,
    dialect: Dialect,
    diodes: &BTreeMap<String, Diode>,
) -> Result<OperatingPoint, DaeError> {
    let nodes = topology::diode_nodes(source, dialect);

    // Fix every diode's conductance at its canonical reference slope and its Norton current at
    // zero: this is the "A0" the whole LCP fold above is built on.
    let mut base_values = BTreeMap::new();
    for (name, diode) in diodes {
        base_values.insert(format!("{name}_G"), diode.g_off);
        base_values.insert(format!("{name}_Ioff"), 0.0);
    }
    let numeric0 = system.evaluate(&base_values).map_err(DaeError::Evaluate)?;
    let x0 = dense_solve(&numeric0.a, &numeric0.u).map_err(DaeError::Linear)?;

    struct DiodeInfo {
        name: String,
        canonical: pwl_devices::DiodeCanonical,
        p: Option<usize>,
        n: Option<usize>,
        w: Vec<f64>,
    }

    let mut infos = Vec::with_capacity(diodes.len());
    for (name, diode) in diodes {
        let column = system
            .inputs
            .iter()
            .position(|input| input == name)
            .ok_or_else(|| DaeError::UnknownDiodeInput(name.clone()))?;
        let b_col: Vec<f64> = (0..system.order())
            .map(|row| numeric0.b[(row, column)])
            .collect();
        let w = dense_solve(&numeric0.a, &b_col).map_err(DaeError::Linear)?;

        let terminals = &nodes[name];
        let p = topology::node_index(&system.unknowns, &terminals.positive);
        let n = topology::node_index(&system.unknowns, &terminals.negative);

        infos.push(DiodeInfo {
            name: name.clone(),
            canonical: diode.canonical(),
            p,
            n,
            w,
        });
    }

    let get = |x: &[f64], index: Option<usize>| index.map(|i| x[i]).unwrap_or(0.0);

    let n_diodes = infos.len();
    let v0: Vec<f64> = infos
        .iter()
        .map(|d| get(&x0, d.p) - get(&x0, d.n))
        .collect();
    // gamma[k][j] = w_j[p_k] - w_j[n_k]: how much diode j's unit raw current moves diode k's
    // own terminal voltage (the diagonal, gamma[k][k], is the self term; everything else is
    // genuine cross-coupling through the shared linear network).
    let gamma: Vec<Vec<f64>> = infos
        .iter()
        .map(|k| {
            infos
                .iter()
                .map(|j| get(&j.w, k.p) - get(&j.w, k.n))
                .collect()
        })
        .collect();

    let dim = 2 * n_diodes;
    let mut m = vec![vec![0.0; dim]; dim];
    let mut q = vec![0.0; dim];
    for k in 0..n_diodes {
        let row_w1 = 2 * k;
        q[row_w1] = v0[k] - infos[k].canonical.v_breakdown;
        for (j, info_j) in infos.iter().enumerate() {
            m[row_w1][2 * j] += -gamma[k][j] * info_j.canonical.delta_br;
            m[row_w1][2 * j + 1] += gamma[k][j] * info_j.canonical.delta_on;
        }
        m[row_w1][2 * k] += 1.0;

        let row_w2 = 2 * k + 1;
        q[row_w2] = infos[k].canonical.v_th - v0[k];
        for (j, info_j) in infos.iter().enumerate() {
            m[row_w2][2 * j] += gamma[k][j] * info_j.canonical.delta_br;
            m[row_w2][2 * j + 1] += -gamma[k][j] * info_j.canonical.delta_on;
        }
        m[row_w2][2 * k + 1] += 1.0;
    }

    let sol = lcp_solver::solve(&m, &q).map_err(DaeError::Lcp)?;

    let mut x = x0;
    let mut diode_z = Vec::with_capacity(n_diodes);
    for (k, info) in infos.iter().enumerate() {
        let z1 = sol.z[2 * k];
        let z2 = sol.z[2 * k + 1];
        let raw_ioff = info.canonical.delta_on * z2 - info.canonical.delta_br * z1;
        for (xi, wi) in x.iter_mut().zip(info.w.iter()) {
            *xi += wi * raw_ioff;
        }
        diode_z.push((z1, z2));
    }

    Ok(OperatingPoint {
        unknowns: system.unknowns.clone(),
        x,
        diode_names: infos.into_iter().map(|d| d.name).collect(),
        diode_z,
    })
}
