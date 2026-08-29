"""err1/err2/error(l1/l2/infnorm) coverage. See fixtures/err.cir's own header comment for the
hand-derivation (measured always exactly double the comparison => err1=err2=0.5; a constant +3
offset => L1=L2=InfNorm=3 exactly)."""
import sys
import pathlib

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from _lib import run_measure, parse_measurements, assert_close  # noqa: E402


def test_err_measurements():
    _, _, stderr = run_measure("err.cir", tfinal=10, dt=0.01)
    m = parse_measurements(stderr)
    assert_close(m["MEAS_ERR1"], 0.5, 1e-6, "err1")
    assert_close(m["MEAS_ERR2"], 0.5, 1e-6, "err2")
    assert_close(m["MEAS_L1"], 3.0, 1e-6, "error norm=l1")
    assert_close(m["MEAS_L2"], 3.0, 1e-6, "error norm=l2")
    assert_close(m["MEAS_INF"], 3.0, 1e-6, "error norm=infnorm")


if __name__ == "__main__":
    test_err_measurements()
    print("ok")
