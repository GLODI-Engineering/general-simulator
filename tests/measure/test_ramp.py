"""Scalar/crossing kind=measure coverage: max, min, max_at, min_at, pp, avg, rms, integ, deriv
(at= and when= forms), find (at= and when= forms), when, trig_targ. See fixtures/ramp.cir's own
header comment for the hand-derivation of every expected value below.
"""
import sys
import pathlib

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from _lib import run_measure, parse_measurements, assert_close  # noqa: E402

DT = 0.01


def test_ramp_measurements():
    _, _, stderr = run_measure("ramp.cir", tfinal=10, dt=DT)
    m = parse_measurements(stderr)

    # The resolved trace's first row is t=dt (the first *completed* step), not t=0 -- see
    # ramp.cir's own header comment -- so min/min_at land on RAMP(dt)=2*dt / dt, not on 0.
    assert_close(m["MEAS_MAX"], 20.0, 1e-6, "max")
    assert_close(m["MEAS_MIN"], 2 * DT, 1e-6, "min")
    assert_close(m["MEAS_MAXAT"], 10.0, 1e-6, "max_at")
    assert_close(m["MEAS_MINAT"], DT, 1e-6, "min_at")
    assert_close(m["MEAS_PP"], 20.0 - 2 * DT, 1e-6, "pp")
    assert_close(m["MEAS_AVG"], 10.0, 0.02, "avg")
    assert_close(m["MEAS_RMS"], (400.0 / 3.0) ** 0.5, 0.02, "rms")
    assert_close(m["MEAS_INTEG"], 100.0, 0.01, "integ")
    assert_close(m["MEAS_DERIVAT"], 2.0, 1e-6, "deriv at=")
    assert_close(m["MEAS_FINDAT"], 10.0, 1e-6, "find at=")
    assert_close(m["MEAS_WHEN"], 5.0, 1e-6, "when")
    assert_close(m["MEAS_FINDWHEN"], 150.0, 1e-6, "find when=")
    assert_close(m["MEAS_DERIVWHEN"], 2.0, 1e-6, "deriv when=")
    assert_close(m["MEAS_TRIGTARG"], 6.5, 1e-6, "trig_targ")


if __name__ == "__main__":
    test_ramp_measurements()
    print("ok")
