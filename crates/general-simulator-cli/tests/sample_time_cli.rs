//! Runs the actual built `general-simulator` binary against `kind=cscript` netlists exercising
//! the two new sample-time features -- block-controlled variable scheduling (`ts=variable`,
//! `cscript_next_sample_hit`) and a fixed-period phase offset (`ts=`/`to=`) -- an end-to-end
//! confirmation (real CLI, real netlist, real block graph) that both work through the whole
//! stack, not just `cscript-ffi`'s own lower-level unit tests. No dummy ideal switch needed: the
//! block graph runs whenever a block is declared, ideal switch or not.

use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn compile_c_fixture(name: &str) -> PathBuf {
    let source = fixture(&format!("{name}.c"));
    let out_dir = std::env::temp_dir().join("general-simulator-cli-test-fixtures");
    std::fs::create_dir_all(&out_dir).expect("create fixture output dir");
    let lib_path = out_dir.join(format!("lib{name}.so"));
    let status = Command::new("cc")
        .args(["-shared", "-fPIC", "-O0", "-o"])
        .arg(&lib_path)
        .arg(&source)
        .status()
        .expect("run cc to compile fixture");
    assert!(status.success(), "cc failed to compile fixture {name}");
    lib_path
}

fn write_devices_file(contents: &str) -> PathBuf {
    let out_dir = std::env::temp_dir().join("general-simulator-cli-test-fixtures");
    std::fs::create_dir_all(&out_dir).expect("create fixture output dir");
    let path = out_dir.join(format!(
        "devices-sampletime-{}.txt",
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

fn run_and_read_last_column(devices: &str, tfinal: &str, dt: &str) -> Vec<(f64, f64)> {
    let netlist = write_devices_file("V1 a 0 5\nR1 a 0 1000\n");
    let devices_path = write_devices_file(devices);
    let output = run(&[
        netlist.to_str().unwrap(),
        "--devices",
        devices_path.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        tfinal,
        "--dt",
        dt,
    ]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    stdout
        .lines()
        .skip(1)
        .map(|l| {
            let mut cols = l.split(',');
            let t: f64 = cols.next().unwrap().parse().unwrap();
            let v: f64 = cols.next_back().unwrap().parse().unwrap();
            (t, v)
        })
        .collect()
}

#[test]
fn ts_variable_follows_the_block_own_doubling_schedule() {
    let lib = compile_c_fixture("variable_sample_time");
    let devices = format!(
        "SRC kind=const value=1\n\
         CNT kind=cscript lib={} in=SRC ts=variable\n",
        lib.display()
    );
    let rows = run_and_read_last_column(&devices, "20.0", "1.0");

    // With dt=1.0, evaluate_blocks calls land at t=1,2,3,4,...; the fixture's own next-hit
    // schedule (1,2,4,8,16,...) means output() actually runs (call count increments) at
    // t=1,2,4,8,16 and holds its value in between -- confirmed against every logged row, not
    // just the hit points, so the zero-order-hold behavior between hits is checked too.
    let value_at = |t_target: f64| -> f64 {
        rows.iter()
            .find(|(t, _)| (*t - t_target).abs() < 1e-9)
            .unwrap_or_else(|| panic!("no row at t={t_target}, rows={rows:?}"))
            .1
    };
    assert_eq!(value_at(1.0), 1.0);
    assert_eq!(value_at(2.0), 2.0);
    assert_eq!(value_at(3.0), 2.0); // held -- not due again until t=4
    assert_eq!(value_at(4.0), 3.0);
    assert_eq!(value_at(7.0), 3.0); // held -- not due again until t=8
    assert_eq!(value_at(8.0), 4.0);
    assert_eq!(value_at(15.0), 4.0); // held -- not due again until t=16
    assert_eq!(value_at(16.0), 5.0);
    assert_eq!(value_at(20.0), 5.0); // held -- next hit (t=32) is past tfinal
}

#[test]
fn ts_offset_delays_the_first_hit_without_changing_the_period() {
    let lib = compile_c_fixture("call_counter");
    // period=2.0, offset=0.7 -- hits expected at t=0.7, 2.7, 4.7.
    let devices = format!(
        "SRC kind=const value=0\n\
         CNT kind=cscript lib={} in=SRC ts=2.0 to=0.7\n",
        lib.display()
    );
    let rows = run_and_read_last_column(&devices, "5.0", "0.1");

    let value_at = |t_target: f64| -> f64 {
        rows.iter()
            .find(|(t, _)| (*t - t_target).abs() < 1e-6)
            .unwrap_or_else(|| panic!("no row at t={t_target}, rows={rows:?}"))
            .1
    };
    assert_eq!(value_at(0.6), 0.0); // not due yet
    assert_eq!(value_at(0.7), 1.0); // first hit, exactly at the requested offset
    assert_eq!(value_at(2.6), 1.0); // held
    assert_eq!(value_at(2.7), 2.0); // second hit, one full period after the first
    assert_eq!(value_at(4.7), 3.0); // third hit
}

#[test]
fn ts_offset_omitted_reproduces_the_pre_existing_immediate_first_hit_behavior() {
    // No `to=` at all -- must behave exactly like it always did: due on the very first
    // evaluate_blocks call, not delayed by a full period. This is the mandatory backward-
    // compatibility case: every `ts=`-only netlist in this workspace (and an internal sibling
    // archive of validation netlists) depends on this not changing.
    let lib = compile_c_fixture("call_counter");
    let devices = format!(
        "SRC kind=const value=0\n\
         CNT kind=cscript lib={} in=SRC ts=2.0\n",
        lib.display()
    );
    let rows = run_and_read_last_column(&devices, "5.0", "0.1");
    let value_at = |t_target: f64| -> f64 {
        rows.iter()
            .find(|(t, _)| (*t - t_target).abs() < 1e-6)
            .unwrap_or_else(|| panic!("no row at t={t_target}, rows={rows:?}"))
            .1
    };
    assert_eq!(value_at(0.1), 1.0); // due immediately, on the very first resolved step
    assert_eq!(value_at(2.0), 1.0); // held until the next full period
    assert_eq!(value_at(2.1), 2.0); // second hit, one period after the first
}
