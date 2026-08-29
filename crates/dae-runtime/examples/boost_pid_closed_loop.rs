//! Same closed-loop boost-PID scenario as
//! `crates/dae-runtime/tests/closed_loop_boost_anti_windup.rs` (see that file's doc comment
//! for the full context: two-sided anti-windup vs. the documented Xyce/ngspice windup-collapse
//! failure), run as an example instead of a test so its full transient trace can be exported
//! for plotting. See
//! `internal-archive/experiments/elspice-pwl-boost-llc-vs-xyce-ngspice/README.md`.

use std::collections::BTreeMap;
use std::io::Write;

use continuous_blocks::Pid;
use dae_runtime::{sawtooth_carrier, simulate_closed_loop, GateState};
use general_spice_core::Dialect;
use pwl_devices::{IdealDiode, IdealSwitch};

fn main() {
    let netlist = "V1 vin 0 12\nD1 vx 0 idealswitchmodel\nD2 vx vout dmodel\nL1 vin vx 100u\nC1 vout 0 100u\nR1 vout 0 50";
    let switch = IdealSwitch::new(0.01, IdealDiode::new(0.0, -1e6, 0.0, 1e6, 0.0));
    let mut ideal_switches = BTreeMap::new();
    ideal_switches.insert("D1".to_string(), switch);
    let mut diodes = BTreeMap::new();
    diodes.insert(
        "D2".to_string(),
        IdealDiode::new(0.0, -100.0, 0.0, 0.6, 100.0),
    );

    let reference = 24.0;
    let pid = Pid::new(0.01, 20.0, 0.0, 1000.0).unwrap();
    let controller = pid.to_state_space();

    let switching_freq = 100_000.0;
    let measure = |point: &dae_runtime::OperatingPoint| point.value("V(vout)").unwrap();
    let pwm = move |duty: f64, t: f64| {
        let duty = duty.clamp(0.0, 1.0);
        let carrier = sawtooth_carrier(t, switching_freq);
        let state = if carrier < duty {
            GateState::On
        } else {
            GateState::Off
        };
        BTreeMap::from([("D1".to_string(), state)])
    };

    let dt = 1e-7;
    let t_final = 0.010;

    let trace = simulate_closed_loop(
        netlist,
        Dialect::Ngspice,
        &diodes,
        &ideal_switches,
        &controller,
        move |_| reference,
        measure,
        pwm,
        (0.0, 1.0),
        0.01,
        None,
        t_final,
        dt,
    )
    .unwrap();

    let out_path = "boost_pid_elspice_pwl_out.csv";
    let mut f = std::fs::File::create(out_path).unwrap();
    writeln!(f, "t,V(vout),duty").unwrap();
    for (t, point, duty) in &trace {
        writeln!(f, "{t},{},{}", point.value("V(vout)").unwrap(), duty).unwrap();
    }
    println!("wrote {out_path}");
}
