"""Cycle-counting kind=measure coverage: freq, on_time, off_time. See fixtures/square.cir's own
header comment for the hand-derivation (1.0 Hz, 0.5s/0.5s on/off, 50% duty)."""
import sys
import pathlib

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from _lib import run_measure, parse_measurements, assert_close  # noqa: E402


def test_square_measurements():
    _, _, stderr = run_measure("square.cir", tfinal=4.0, dt=0.0001)
    m = parse_measurements(stderr)
    assert_close(m["MEAS_FREQ"], 1.0, 0.01, "freq")
    assert_close(m["MEAS_ONTIME"], 0.5, 0.01, "on_time")
    assert_close(m["MEAS_OFFTIME"], 0.5, 0.01, "off_time")


if __name__ == "__main__":
    test_square_measurements()
    print("ok")
