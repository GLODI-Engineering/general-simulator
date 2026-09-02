//! Runs the actual built `general-simulator` binary against a netlist demonstrating the real
//! motivating use case for this whole block family: a sticky fault latch, aggregated from two
//! independent trip conditions through an `or` gate, gating a real ideal switch through the existing
//! `sig2phys domain=voltage` Signal-to-PS boundary -- end-to-end confirmation (real CLI, real netlist, real
//! electrical circuit) that logic blocks compose with the rest of the signal domain, not just
//! with each other in isolation.

use std::path::PathBuf;
use std::process::Command;

fn write_devices_file(contents: &str) -> PathBuf {
    let out_dir = std::env::temp_dir().join("general-simulator-cli-test-fixtures");
    std::fs::create_dir_all(&out_dir).expect("create fixture output dir");
    let path = out_dir.join(format!(
        "devices-logic-{}.txt",
        std::process::id() as u64 * 1000 + contents.len() as u64
    ));
    std::fs::write(&path, contents).expect("write devices file");
    path
}

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_general-simulator"))
        .args(args)
        .output()
        .expect("failed to run general-simulator binary")
}

#[test]
fn an_or_aggregated_fault_latch_stays_tripped_and_blocks_the_ideal_switch_even_after_the_fault_clears(
) {
    let netlist = write_devices_file("V1 a 0 5\nD1 a b idealswitchmodel\nR1 b 0 1000\n");
    let devices = write_devices_file(
        "FAULT1 kind=pwc points=[[0,0],[0.001,1],[0.0015,0]]\n\
         FAULT2 kind=const value=0\n\
         TRIP kind=or inputs=FAULT1,FAULT2\n\
         RESETCMD kind=const value=0\n\
         LATCH kind=srlatch set=TRIP reset=RESETCMD\n\
         ENABLE kind=not in=LATCH\n\
         GATE_V kind=sig2phys domain=voltage in=ENABLE\n\
         D1 kind=ideal_switch r_on=0.1 g_breakdown=0 v_breakdown=-100 g_off=0 v_th=0.7 g_on=1 \
         gate=block ctrl=GATE_V\n",
    );

    let output = run(&[
        netlist.to_str().unwrap(),
        "--devices",
        devices.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "0.003",
        "--dt",
        "0.0005",
    ]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    let header: Vec<&str> = lines[0].split(',').collect();
    let t_idx = header.iter().position(|h| *h == "t").unwrap();
    let vb_idx = header.iter().position(|h| *h == "V(b)").unwrap();
    let latch_idx = header.iter().position(|h| *h == "LATCH").unwrap();

    let row_at = |t_target: f64| -> (f64, f64) {
        lines[1..]
            .iter()
            .map(|l| l.split(',').collect::<Vec<_>>())
            .find(|f| (f[t_idx].parse::<f64>().unwrap() - t_target).abs() < 1e-9)
            .map(|f| (f[latch_idx].parse().unwrap(), f[vb_idx].parse().unwrap()))
            .unwrap_or_else(|| panic!("no row at t={t_target}"))
    };

    // Before the fault: latch clear, ideal switch conducting (R_on=0.1 vs. R1=1000 -> V(b) close to
    // 5V, a real voltage divider, not just "some nonzero value").
    let (latch_before, vb_before) = row_at(0.0005);
    assert_eq!(latch_before, 0.0);
    assert!(
        vb_before > 4.0,
        "expected ideal switch conducting, V(b)={vb_before}"
    );

    // During the fault pulse: latch trips, ideal switch blocks (V(b) collapses toward 0).
    let (latch_during, vb_during) = row_at(0.0010);
    assert_eq!(latch_during, 1.0);
    assert!(
        vb_during < 0.1,
        "expected ideal switch blocked, V(b)={vb_during}"
    );

    // After the fault pulse clears (FAULT1 back to 0): latch stays latched -- the whole point
    // of a *sticky* fault latch -- ideal switch stays blocked, not just momentarily during the pulse.
    let (latch_after, vb_after) = row_at(0.0025);
    assert_eq!(latch_after, 1.0);
    assert!(
        vb_after < 0.1,
        "expected ideal switch still blocked, V(b)={vb_after}"
    );
}
