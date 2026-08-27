//! The measurements cited in `book/dev-guide/src/python-blocks.md`'s own "Measured, not
//! assumed" section were produced by this exact program (`cargo run --release --example
//! bench`) -- kept committed so the numbers can be independently re-verified rather than taken
//! on faith, and re-run if `pyo3`/`numpy` versions change enough to move them.

use std::time::Instant;

use pyo3::prelude::*;
use pyo3::types::PyDict;

fn main() -> PyResult<()> {
    Python::attach(|py| {
        let globals = PyDict::new(py);
        py.run(
            pyo3::ffi::c_str!("def output(t, dt, u):\n    return u[0] * 2.0 + u[1]\n"),
            Some(&globals),
            None,
        )?;
        let func = globals.get_item("output")?.unwrap();

        // Warm up.
        for _ in 0..1000 {
            let _: f64 = func.call1((0.0, 1e-5, (1.0, 2.0)))?.extract()?;
        }

        let n = 200_000;
        let start = Instant::now();
        let mut acc = 0.0;
        for i in 0..n {
            let r: f64 = func.call1((0.0, 1e-5, (i as f64, 2.0)))?.extract()?;
            acc += r;
        }
        let elapsed = start.elapsed();
        println!(
            "plain scalar call: {n} calls in {:?} -> {:.1} ns/call (sink={acc})",
            elapsed,
            elapsed.as_nanos() as f64 / n as f64
        );

        // numpy round trip.
        let np = py.import("numpy")?;
        let globals2 = PyDict::new(py);
        globals2.set_item("np", &np)?;
        py.run(
            pyo3::ffi::c_str!("def output_np(t, dt, u):\n    return u * 2.0\n"),
            Some(&globals2),
            None,
        )?;
        let func_np = globals2.get_item("output_np")?.unwrap();
        let arr = numpy::PyArray1::from_vec(py, vec![1.0_f64, 2.0, 3.0]);
        for _ in 0..1000 {
            let _ = func_np.call1((0.0, 1e-5, &arr))?;
        }
        let start2 = Instant::now();
        for _ in 0..n {
            let _ = func_np.call1((0.0, 1e-5, &arr))?;
        }
        let elapsed2 = start2.elapsed();
        println!(
            "numpy array call: {n} calls in {:?} -> {:.1} ns/call",
            elapsed2,
            elapsed2.as_nanos() as f64 / n as f64
        );

        Ok::<(), PyErr>(())
    })?;

    // Re-acquiring the GIL fresh every call (Python::attach per iteration) vs. holding it once
    // across the whole loop -- the real question for how the transient loop should structure
    // its own Python-block calls.
    let n = 200_000;
    let start3 = Instant::now();
    for _ in 0..n {
        Python::attach(|_py| {});
    }
    let elapsed3 = start3.elapsed();
    println!(
        "bare Python::attach (no call): {n} in {:?} -> {:.1} ns/call",
        elapsed3,
        elapsed3.as_nanos() as f64 / n as f64
    );
    Ok(())
}
