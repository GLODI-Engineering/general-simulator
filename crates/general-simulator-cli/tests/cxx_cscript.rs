//! Runs the actual built `general-simulator` binary against a netlist using `kind=cscript`
//! where the `.so` was compiled from C++ (via `cscript-ffi`'s own `cscript.hpp` convenience
//! header) instead of plain C -- an end-to-end confirmation that a C++-authored library is a
//! fully interchangeable `kind=cscript` target through the real CLI, needing zero changes on
//! any Rust side (the netlist keyword is still `kind=cscript`, `lib=` still just names a `.so`
//! `dlopen` doesn't care what compiled). No dummy MOSFET is needed here (unlike `cscript.rs`'s
//! own older tests): `general-simulator-cli` now runs the block graph whenever a block is
//! declared at all, MOSFET or not.

use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn compile_fixture(name: &str) -> PathBuf {
    let source = fixture(&format!("{name}.cpp"));
    let out_dir = std::env::temp_dir().join("general-simulator-cli-test-fixtures");
    std::fs::create_dir_all(&out_dir).expect("create fixture output dir");
    let lib_path = out_dir.join(format!("lib{name}.so"));
    let status = Command::new("c++")
        .args(["-std=c++17", "-shared", "-fPIC", "-O0", "-o"])
        .arg(&lib_path)
        .arg(&source)
        .status()
        .expect("run c++ to compile fixture");
    assert!(status.success(), "c++ failed to compile fixture {name}");
    lib_path
}

fn write_devices_file(contents: &str) -> PathBuf {
    let out_dir = std::env::temp_dir().join("general-simulator-cli-test-fixtures");
    std::fs::create_dir_all(&out_dir).expect("create fixture output dir");
    let path = out_dir.join(format!(
        "devices-cxx-{}.txt",
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
fn a_cxx_authored_cscript_library_reproduces_the_same_result_as_the_c_one() {
    let lib = compile_fixture("cxx_cscript_gain");
    let netlist = write_devices_file("V1 a 0 5\nR1 a 0 1000\n");
    let devices = write_devices_file(&format!(
        "SRC kind=const value=5\n\
         CG kind=cscript lib={} in=SRC\n",
        lib.display()
    ));

    let output = run(&[
        netlist.to_str().unwrap(),
        "--devices",
        devices.to_str().unwrap(),
        "--mode",
        "transient",
        "--tfinal",
        "0.001",
        "--dt",
        "0.0002",
    ]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(lines[0].ends_with(",CG"), "header: {}", lines[0]);
    for line in &lines[1..] {
        let cg: f64 = line.split(',').next_back().unwrap().parse().unwrap();
        assert!((cg - 10.0).abs() < 1e-12, "expected CG=10 (5*2), got {cg}");
    }
}
